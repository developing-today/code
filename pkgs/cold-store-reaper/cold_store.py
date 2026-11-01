#!/usr/bin/env python3
"""cold-store reaper v1: archive files into Google Drive as immutable,
content-addressed bundles of concatenated per-object zstd frames.

Stdlib only. Compression/decompression is delegated to the `zstd` CLI,
and S3 downloads that urllib cannot deliver whole fall back to the
`curl` CLI (see s3_get_object).

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
import hmac
import http.client
import json
import mimetypes
import os
import re
import sqlite3
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
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

# DB-backup class (DESIGN.md section 5): nightly dumps live unbundled under
# db-backups/<db>/, one subfolder per database.
BACKUP_DBS = ("supabase", "turso", "d1")

# Manifest sync target (DESIGN.md section 3): the hosted copy of this schema lives
# in Cloudflare D1. The token comes from the environment only; the account
# and database IDs are non-secret and default to the cold-store database.
DEFAULT_D1_BASE = "https://api.cloudflare.com/client/v4"
DEFAULT_CF_ACCOUNT_ID = "234b866b99ce2b142d7963d8d139cbfa"
DEFAULT_CF_D1_DATABASE_ID = "15d5f460-835c-4f24-b229-222908a5b30c"
MANIFEST_TABLES = ("objects", "bundles", "members", "transfer_ledger", "meta")
# D1 allows at most 100 bound parameters per query. A sync statement batches
# whole rows, so it costs rows x columns parameters: a flat row batch (50
# rows x 8 columns for objects = 400 params) is rejected once a table grows.
# Batch size is therefore computed per table from its column count against
# a 90-parameter budget, keeping headroom under the hard limit.
D1_MAX_PARAMS_PER_QUERY = 100
D1_SYNC_PARAM_BUDGET = 90


# R2 (DESIGN.md sections 4-5): the supabase-backups bucket is both a
# nightly filing source (backup-supabase) and the first eviction trial
# bucket. Access is the S3 API at the account's R2 host, with
# bucket-scoped credentials from the environment only.
DEFAULT_R2_HOST = "234b866b99ce2b142d7963d8d139cbfa.r2.cloudflarestorage.com"
DEFAULT_R2_BUCKET = "supabase-backups"
R2_SIGNING_REGION = "auto"  # R2 is region-agnostic; SigV4 still needs one
S3_GET_ATTEMPTS = 3
# The curl fallback for S3 downloads (s3_get_object) is a last resort
# for objects urllib cannot deliver whole; it gets one attempt and a
# generous timeout — the objects it exists for are the large ones.
S3_CURL_TIMEOUT_SECONDS = 900
# Transport-level retries inside the S3 layer (LIST/GET/DELETE are all
# idempotent). Through some egress proxies, urllib responses are
# intermittently truncated mid-body or the connection drops outright;
# http_request surfaces those as the exceptions below, and before this
# retry existed they escaped _s3_request as raw tracebacks that killed
# the caller at the LIST step. Only S3 requests retry — Drive and the
# other http_request callers keep their own semantics.
S3_TRANSPORT_ATTEMPTS = 4
S3_TRANSPORT_BACKOFF_SECONDS = 0.5  # x attempt number, capped below
S3_TRANSPORT_BACKOFF_CAP_SECONDS = 5.0
# A reset during the request-send phase is wrapped by urllib into a
# URLError, which http_request converts to ColdStoreError; _s3_request
# adds ColdStoreError to this tuple at call time (the class is defined
# further down). Its try covers only the http_request call, so no
# other ColdStoreError can be caught by the retry.
S3_TRANSPORT_ERRORS = (
    http.client.IncompleteRead,
    http.client.RemoteDisconnected,
    http.client.BadStatusLine,
    ConnectionResetError,
)


def s3_error_message(body: bytes) -> str:
    """Best-effort <Message> extraction from an S3 XML error body."""
    m = re.search(rb"<Message>([^<]+)</Message>", body or b"")
    if m:
        return m.group(1).decode("utf-8", "replace")
    text = (body or b"").decode("utf-8", "replace").strip()
    return text[:200] if text else "no error detail in response body"


def _scrub_secrets(text: str, *values: str) -> str:
    """Replace credential values in text destined for an error
    message. The curl fallback passes credentials in argv (visible in
    the local process list, like any CLI); they must still never land
    in logs or error text."""
    for v in values:
        if v:
            text = text.replace(v, "***")
    return text


def _header_value(headers: dict, name: str):
    for k, v in headers.items():
        if k.lower() == name.lower():
            return v
    return None


def _xml_local(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def _parse_s3_list_page(body: bytes):
    """One ListObjectsV2 page -> (objects, is_truncated, next_token).

    Namespace-agnostic (S3 replies carry a default xmlns; a stub may
    omit it). Each object is {key, size, last_modified}; last_modified
    is an aware datetime, or None when absent/unparseable — callers
    treat None as oldest, never as 'now'.
    """
    try:
        root = ET.fromstring(body)
    except ET.ParseError as exc:
        raise ColdStoreError(
            f"s3 list page is not well-formed XML: {exc}"
        ) from exc
    if _xml_local(root.tag) != "ListBucketResult":
        raise ColdStoreError(
            "s3 list page root is "
            f"{_xml_local(root.tag)!r}, expected 'ListBucketResult' "
            "(refusing to read a non-listing body as an empty bucket)"
        )
    objects = []
    truncated = False
    token = None
    for child in root:
        name = _xml_local(child.tag)
        if name == "IsTruncated":
            truncated = (child.text or "").strip().lower() == "true"
        elif name == "NextContinuationToken":
            token = child.text
        elif name == "Contents":
            fields = {_xml_local(c.tag): c.text for c in child}
            stamp = None
            raw = fields.get("LastModified")
            if raw:
                try:
                    stamp = datetime.fromisoformat(raw.replace("Z", "+00:00"))
                except ValueError:
                    stamp = None
                if stamp is not None and stamp.tzinfo is None:
                    # Offset-less = zone unknown = unparseable (the
                    # aware-or-None contract above): never reinterpret
                    # it as UTC, because eviction deletes objects and
                    # must not act on a stamp whose zone is unknown.
                    # None reads as oldest/too-young downstream, which
                    # is non-evictable by construction (#2110).
                    stamp = None
            raw_size = fields.get("Size")
            if raw_size is None or raw_size.strip() == "":
                size = 0
            else:
                try:
                    size = int(raw_size.strip())
                except ValueError:
                    raise ColdStoreError(
                        "s3 list page Contents entry "
                        f"{fields.get('Key') or ''!r} has a non-numeric "
                        f"Size {raw_size!r}"
                    ) from None
                if size < 0:
                    raise ColdStoreError(
                        "s3 list page Contents entry "
                        f"{fields.get('Key') or ''!r} has a negative "
                        f"Size {raw_size!r}"
                    )
            objects.append(
                {
                    "key": fields.get("Key") or "",
                    "size": size,
                    "last_modified": stamp,
                }
            )
    return objects, truncated, token


def content_hash_from_key(key: str) -> str | None:
    """The content sha256 a content-addressed R2 key carries, or None.

    Trial-bucket keys look like pg/muse-dev/2026/10/<sha256>.api.jsonl.zst:
    the basename's first dot-segment is the hash when it is 64-hex —
    the same hash the manifest keys objects by.
    """
    stem = key.rsplit("/", 1)[-1].split(".", 1)[0]
    if re.fullmatch(r"[0-9a-f]{64}", stem):
        return stem
    return None


def d1_sync_batch_rows(ncols: int) -> int:
    """Rows per sync statement for a table with ncols columns."""
    return max(1, D1_SYNC_PARAM_BUDGET // ncols)


def d1_error_message(resp: bytes) -> str:
    """Best-effort extraction of the D1 API's own error text from a
    response body, so a sync failure is diagnosable from the CLI output
    alone instead of just an HTTP status."""
    try:
        payload = json.loads(resp) if resp else {}
    except (ValueError, TypeError):
        payload = {}
    errors = payload.get("errors") or []
    if errors and errors[0].get("message"):
        return str(errors[0]["message"])
    text = (resp or b"").decode("utf-8", "replace").strip()
    return text[:200] if text else "no error detail in response body"


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
        self,
        home: Path,
        token_url=None,
        drive_base=None,
        upload_base=None,
        zstd=None,
        d1_base=None,
        s3_base=None,
        curl=None,
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
        self.d1_base = (
            d1_base or os.environ.get("COLD_STORE_D1_BASE", DEFAULT_D1_BASE)
        ).rstrip("/")
        self.s3_base = (
            s3_base
            or os.environ.get("COLD_STORE_S3_BASE")
            or ("https://" + os.environ.get("COLD_STORE_R2_HOST", DEFAULT_R2_HOST))
        ).rstrip("/")
        candidate = zstd or os.environ.get("COLD_STORE_ZSTD") or "/usr/local/bin/zstd"
        if not Path(candidate).exists():
            candidate = "zstd"  # fall back to PATH lookup
        self.zstd = candidate
        # curl is the S3 download fallback of last resort
        # (s3_get_object); PATH lookup unless overridden, like zstd.
        self.curl = curl or os.environ.get("COLD_STORE_CURL") or "curl"
        self._access_token = None
        self._access_token_expires_at = 0.0
        # Injectable so tests can zero it: transport-retry backoff for
        # the S3 layer (see _s3_request).
        self.s3_backoff_seconds = S3_TRANSPORT_BACKOFF_SECONDS

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
            self._write_private_json(self.state_path, {})
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
        # state.json is written 0600 like credentials.json, and it must
        # never carry credential material: drop any access_token a
        # pre-#940 state file may still hold (tokens live in memory only).
        state = {k: v for k, v in state.items() if k != "access_token"}
        self._write_private_json(self.state_path, state)

    # ------------------------------------------------------------------- db
    def db(self) -> sqlite3.Connection:
        conn = sqlite3.connect(self.manifest_path)
        conn.row_factory = sqlite3.Row
        conn.execute("PRAGMA foreign_keys = ON")
        if not self.manifest_path.exists() or self._schema_missing(conn):
            conn.executescript(SCHEMA_PATH.read_text())
            conn.commit()
        self._migrate_ledger_trash(conn)
        self._migrate_objects_evicted(conn)
        return conn

    @staticmethod
    def _migrate_ledger_trash(conn) -> None:
        """Let pre-v3 manifests record prune rows: rebuild transfer_ledger
        with the current schema.sql definition if its CHECK predates the
        'trash' direction. No-op for fresh or already-migrated databases."""
        row = conn.execute(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='transfer_ledger'"
        ).fetchone()
        if row is None or "trash" in (row["sql"] or ""):
            return
        text = SCHEMA_PATH.read_text()
        create = re.search(
            r"CREATE TABLE IF NOT EXISTS transfer_ledger \(.*?\);", text, re.S
        )
        index = re.search(r"CREATE INDEX IF NOT EXISTS idx_ledger_ts[^;]*;", text)
        if create is None or index is None:
            raise ColdStoreError("schema.sql is missing the transfer_ledger definition")
        conn.execute("ALTER TABLE transfer_ledger RENAME TO transfer_ledger_pre_trash")
        conn.execute(create.group(0))
        conn.execute(
            "INSERT INTO transfer_ledger (id, ts, direction, bytes, kind, ref)"
            " SELECT id, ts, direction, bytes, kind, ref FROM transfer_ledger_pre_trash"
        )
        conn.execute("DROP TABLE transfer_ledger_pre_trash")
        conn.execute(index.group(0))
        conn.commit()

    @staticmethod
    def _migrate_objects_evicted(conn) -> None:
        """Let pre-v4 manifests record the 'evicted' object state:
        rebuild objects with the current schema.sql CHECK if its CHECK
        predates 'evicted'. members references objects by name, so the
        new table is built under a scratch name, copied into, and
        renamed over the dropped original — with foreign keys off,
        because dropping a referenced parent would otherwise fail.
        No-op for fresh or already-migrated databases."""
        row = conn.execute(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='objects'"
        ).fetchone()
        if row is None or "'evicted'" in (row["sql"] or ""):
            return
        text = SCHEMA_PATH.read_text()
        create = re.search(r"CREATE TABLE IF NOT EXISTS objects \(.*?\);", text, re.S)
        index = re.search(r"CREATE INDEX IF NOT EXISTS idx_objects_state[^;]*;", text)
        if create is None or index is None:
            raise ColdStoreError("schema.sql is missing the objects definition")
        cols = [r["name"] for r in conn.execute("PRAGMA table_info(objects)")]
        collist = ", ".join(cols)
        if conn.in_transaction:
            conn.commit()
        conn.execute("PRAGMA foreign_keys = OFF")
        try:
            conn.execute(create.group(0).replace("objects", "objects_new", 1))
            conn.execute(
                f"INSERT INTO objects_new ({collist}) SELECT {collist} FROM objects"
            )
            conn.execute("DROP TABLE objects")
            conn.execute("ALTER TABLE objects_new RENAME TO objects")
            conn.execute(index.group(0))
            conn.commit()
        finally:
            conn.execute("PRAGMA foreign_keys = ON")

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

    # ------------------------------------------------------- upload limits
    @staticmethod
    def _upload_accounting(conn) -> dict:
        """The guardrail accounting shared by `status` and enforcement
        (#938): rolling-24h upload bytes from the transfer ledger,
        verified cold bytes in Drive, and the two meta limits. One
        implementation, so the readout and the refusal can never
        disagree."""
        up_24h = conn.execute(
            "SELECT coalesce(sum(bytes),0) n FROM transfer_ledger"
            " WHERE direction='upload' AND ts >= datetime('now','-1 day')"
        ).fetchone()["n"]
        cold_bytes = conn.execute(
            "SELECT coalesce(sum(bytes),0) n FROM bundles WHERE state='verified'"
        ).fetchone()["n"]
        # #1028: backups live unbundled in Drive and never enter the
        # bundles table, so bundle bytes alone leave the project budget
        # blind to the one class that grows nightly. The transfer
        # ledger records both sides of that flow — every put_backup
        # writes an upload/backup-upload row and prune_backups writes a
        # trash/backup-prune row per trashed file — so backup bytes in
        # Drive are the all-time difference (the ledger is never
        # pruned; only the daily hard stop above uses a 24h window).
        # A backup trashed manually in Drive, outside prune-backups,
        # is not decremented: the total then overcounts and the budget
        # refuses earlier, never later.
        backup_uploaded = conn.execute(
            "SELECT coalesce(sum(bytes),0) n FROM transfer_ledger"
            " WHERE direction='upload' AND kind='backup-upload'"
        ).fetchone()["n"]
        backup_pruned = conn.execute(
            "SELECT coalesce(sum(bytes),0) n FROM transfer_ledger"
            " WHERE direction='trash' AND kind='backup-prune'"
        ).fetchone()["n"]
        backup_bytes = max(0, backup_uploaded - backup_pruned)
        meta = {
            r["key"]: r["value"] for r in conn.execute("SELECT key, value FROM meta")
        }
        return {
            "uploads_24h_bytes": up_24h,
            "cold_bytes": cold_bytes,
            "backup_bytes": backup_bytes,
            "daily_upload_hard_stop_bytes": int(
                meta.get("daily_upload_hard_stop_bytes", "700000000000")
            ),
            "project_budget_bytes": int(meta.get("project_budget_bytes", "0")),
        }

    def _check_upload_limits(self, conn, nbytes: int, what: str) -> None:
        """Refuse a transfer BEFORE any bytes are uploaded when it would
        cross the daily upload hard stop (rolling-24h ledger uploads +
        this transfer) or the project budget (verified cold bytes +
        backup bytes (#1028) + this transfer). Raises ColdStoreError
        naming the limit, the current usage, and the refused size."""
        acct = self._upload_accounting(conn)
        up_24h = acct["uploads_24h_bytes"]
        hard_stop = acct["daily_upload_hard_stop_bytes"]
        if up_24h + nbytes > hard_stop:
            raise ColdStoreError(
                f"{what} refused: daily upload hard stop — {up_24h} B uploaded "
                f"in the last 24h + this {nbytes} B transfer would cross the "
                f"{hard_stop} B daily_upload_hard_stop_bytes limit"
            )
        cold_bytes = acct["cold_bytes"]
        backup_bytes = acct["backup_bytes"]
        budget = acct["project_budget_bytes"]
        if budget and cold_bytes + backup_bytes + nbytes > budget:
            raise ColdStoreError(
                f"{what} refused: project budget — {cold_bytes} B verified "
                f"cold in Drive + {backup_bytes} B backups in Drive + this "
                f"{nbytes} B transfer would cross the "
                f"{budget} B project_budget_bytes limit"
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
        # The access token lives in memory only — it is never persisted
        # (state.json used to cache it; see #940). It is re-derived from
        # the refresh token in credentials.json whenever the in-memory
        # copy is missing or about to expire.
        now = datetime.now(timezone.utc).timestamp()
        if self._access_token and self._access_token_expires_at > now + 60:
            return self._access_token
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
        self._access_token = token
        self._access_token_expires_at = now + payload.get("expires_in", 3600)
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
        for db in BACKUP_DBS:
            key = f"db-backups/{db}"
            if not folders.get(key):
                folders[key] = self._find_folder(
                    db, folders["db-backups"]
                ) or self._create_folder(db, folders["db-backups"])
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
        seen_hashes = set()
        with open(tmp_bundle, "wb") as bundle_out:
            for name in files:
                src = Path(name)
                if not src.is_file():
                    raise ColdStoreError(f"not a file: {src}")
                obj_hash = sha256_file(src)
                # Members are keyed (bundle_id, object_hash): duplicate
                # content in one pack call would write a second frame no
                # member row can ever point at (INSERT OR IGNORE drops
                # it) while member_count/live_bytes counted it (#963).
                # Store each distinct object exactly once.
                if obj_hash in seen_hashes:
                    continue
                seen_hashes.add(obj_hash)
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
        try:
            self._check_upload_limits(conn, row["bytes"], f"upload of {bundle_id}")
        except ColdStoreError:
            conn.close()
            raise
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

    # ------------------------------------------------------------- backups
    def put_backup(self, db_name: str, file: str) -> dict:
        """Upload one DB dump, unbundled, into db-backups/<db>/ (DESIGN section 5).

        The Drive name is dated (<db>-<YYYYMMDDTHHMMSSZ>-<basename>) so a
        folder listing is already a chronological catalogue. Retention
        (30 daily / 12 weekly / 12 monthly, trash-not-delete) is enforced
        separately by `prune-backups`.
        """
        if db_name not in BACKUP_DBS:
            raise ColdStoreError(
                f"unknown backup db {db_name!r} — expected one of {', '.join(BACKUP_DBS)}"
            )
        src = Path(file)
        if not src.is_file():
            raise ColdStoreError(f"not a file: {src}")
        size = src.stat().st_size
        conn = self.db()
        try:
            self._check_upload_limits(conn, size, f"put-backup {db_name}")
        finally:
            conn.close()
        folder_id = self._folder_id(f"db-backups/{db_name}")
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        name = f"{db_name}-{stamp}-{src.name}"
        file_id = self._resumable_upload(src, name, folder_id)
        conn = self.db()
        with conn:
            self.ledger_add(conn, "upload", size, "backup-upload", name)
        conn.close()
        return {
            "db": db_name,
            "name": name,
            "drive_file_id": file_id,
            "bytes": size,
        }

    def list_backups(self, db_name: str | None = None) -> list[dict]:
        if db_name is not None and db_name not in BACKUP_DBS:
            raise ColdStoreError(
                f"unknown backup db {db_name!r} — expected one of {', '.join(BACKUP_DBS)}"
            )
        dbs = [db_name] if db_name else list(BACKUP_DBS)
        out = []
        for db in dbs:
            folder_id = self._folder_id(f"db-backups/{db}")
            q = f"'{folder_id}' in parents and trashed = false"
            page_token = None
            seen_tokens = set()
            while True:
                params = {
                    "q": q,
                    "fields": "nextPageToken,files(id,name,size,createdTime)",
                    "orderBy": "createdTime desc",
                    "pageSize": 1000,
                }
                if page_token is not None:
                    params["pageToken"] = page_token
                url = self.drive_base + "/files?" + urllib.parse.urlencode(params)
                status, _h, resp = http_request(
                    "GET", url, headers=self._auth_headers()
                )
                if status != 200:
                    raise ColdStoreError(
                        f"files.list for backups failed (HTTP {status})"
                    )
                page = json.loads(resp)
                for f in page.get("files", []):
                    out.append(
                        {
                            "db": db,
                            "id": f["id"],
                            "name": f["name"],
                            "size": int(f.get("size") or 0),
                            "created_time": f.get("createdTime", ""),
                        }
                    )
                next_page_token = page.get("nextPageToken")
                if not next_page_token:
                    break
                if next_page_token in seen_tokens:
                    raise ColdStoreError(
                        "files.list for backups repeated nextPageToken"
                    )
                seen_tokens.add(next_page_token)
                page_token = next_page_token
        # Drive was asked for createdTime desc; sort again so the merged
        # multi-db listing is deterministic even if a backend ignores it.
        out.sort(key=lambda r: r["created_time"], reverse=True)
        return out

    # ------------------------------------------------- backup retention
    @staticmethod
    def _parse_backup_stamp(db_name: str, name: str) -> datetime | None:
        """Timestamp embedded in a dated backup name, or None if the name
        does not follow <db>-<YYYYMMDDTHHMMSSZ>-<basename> for this db."""
        m = re.fullmatch(re.escape(db_name) + r"-(\d{8}T\d{6}Z)-.+", name)
        if not m:
            return None
        try:
            return datetime.strptime(m.group(1), "%Y%m%dT%H%M%SZ").replace(
                tzinfo=timezone.utc
            )
        except ValueError:
            return None

    @staticmethod
    def _retention_plan(files: list[dict]) -> tuple[list[dict], list[dict], list[dict]]:
        """Split one db's backup listing into (keep, prune, unparseable).

        `files` are list_backups rows for a single db, each with a parsed
        `stamp` (datetime) or None. The keep set is the union of: the
        newest backup of each of the last 30 distinct calendar days
        present, the newest of each of the last 12 distinct ISO weeks
        present, and the newest of each of the last 12 distinct calendar
        months present — "present" and "newest" are read off the file set
        itself (filename timestamps, newest-first), never the wall clock,
        so the plan is a pure function of the folder contents. Kept rows
        gain a `reasons` list (daily/weekly/monthly); pruned rows are the
        parseable remainder (reason: excess); unparseable names are never
        pruned, only reported.
        """
        parseable = [f for f in files if f["stamp"] is not None]
        unparseable = [f for f in files if f["stamp"] is None]
        parseable.sort(key=lambda f: (f["stamp"], f["name"]), reverse=True)

        def newest_per(keyfn, limit):
            seen: dict = {}
            for f in parseable:
                key = keyfn(f["stamp"])
                if key not in seen and len(seen) < limit:
                    seen[key] = f
            return seen.values()

        reasons: dict[str, set] = {}

        def mark(kept, reason):
            for f in kept:
                reasons.setdefault(f["id"], set()).add(reason)

        mark(newest_per(lambda ts: ts.date(), 30), "daily")
        mark(newest_per(lambda ts: ts.isocalendar()[:2], 12), "weekly")
        mark(newest_per(lambda ts: (ts.year, ts.month), 12), "monthly")
        keep, prune = [], []
        for f in parseable:
            if f["id"] in reasons:
                keep.append({**f, "reasons": sorted(reasons[f["id"]])})
            else:
                prune.append({**f, "reasons": ["excess"]})
        return keep, prune, unparseable

    def prune_backups(
        self, db_name: str | None = None, apply: bool = False
    ) -> list[dict]:
        """Enforce backup retention (DESIGN section 5): 30 daily /
        12 weekly / 12 monthly, trash-not-delete.

        Dry-run by default: compute and return the plan without touching
        Drive. With apply=True, each prune candidate is trashed via
        files.update {trashed: true} (never permanently deleted) and one
        transfer_ledger row per file is written (direction 'trash',
        kind 'backup-prune', ref = filename).
        """
        if db_name is not None and db_name not in BACKUP_DBS:
            raise ColdStoreError(
                f"unknown backup db {db_name!r} — expected one of {', '.join(BACKUP_DBS)}"
            )
        dbs = [db_name] if db_name else list(BACKUP_DBS)
        plans = []
        for db in dbs:
            rows = self.list_backups(db)
            for r in rows:
                r["stamp"] = self._parse_backup_stamp(db, r["name"])
            keep, prune, unparseable = self._retention_plan(rows)
            plans.append(
                {
                    "db": db,
                    "keep": keep,
                    "prune": prune,
                    "unparseable": unparseable,
                    "applied": apply,
                }
            )
        if apply:
            conn = self.db()
            try:
                for plan in plans:
                    for f in plan["prune"]:
                        status, _h, _resp = http_request(
                            "PATCH",
                            self.drive_base + f"/files/{f['id']}",
                            headers=self._auth_headers(
                                {"Content-Type": "application/json"}
                            ),
                            data=json.dumps({"trashed": True}).encode(),
                        )
                        if status != 200:
                            raise ColdStoreError(
                                f"trash failed for backup {f['name']} (HTTP {status})"
                            )
                        # Per-file commit boundary (#2550): the trash
                        # has already happened in Drive, so this file's
                        # ledger row commits in its own transaction
                        # before the next candidate is attempted. A
                        # mid-run failure can then no longer roll back
                        # rows for files that really were trashed
                        # (trashed files are invisible to list_backups,
                        # so a rolled-back row could never be rewritten
                        # by a retry).
                        with conn:
                            self.ledger_add(
                                conn, "trash", f["size"], "backup-prune", f["name"]
                            )
            finally:
                conn.close()
        for plan in plans:
            for f in plan["keep"] + plan["prune"]:
                del f["stamp"]
            for f in plan["unparseable"]:
                del f["stamp"]
        return plans

    def get_backup(self, db_name: str, name: str, out_path: str) -> dict:
        """Download one backup file, in full, by its exact Drive name."""
        if db_name not in BACKUP_DBS:
            raise ColdStoreError(
                f"unknown backup db {db_name!r} — expected one of {', '.join(BACKUP_DBS)}"
            )
        if "'" in name:
            raise ColdStoreError("backup name must not contain a single quote")
        folder_id = self._folder_id(f"db-backups/{db_name}")
        q = f"'{folder_id}' in parents and name = '{name}' and trashed = false"
        url = (
            self.drive_base
            + "/files?"
            + urllib.parse.urlencode({"q": q, "fields": "files(id,name,size)"})
        )
        status, _h, resp = http_request("GET", url, headers=self._auth_headers())
        if status != 200:
            raise ColdStoreError(f"files.list for get-backup failed (HTTP {status})")
        files = json.loads(resp).get("files", [])
        if not files:
            raise ColdStoreError(f"backup not found: {db_name}/{name}")
        file_id = files[0]["id"]
        status, _h, resp = http_request(
            "GET",
            self.drive_base + f"/files/{file_id}?alt=media",
            headers=self._auth_headers(),
        )
        if status != 200:
            raise ColdStoreError(f"backup download failed (HTTP {status})")
        Path(out_path).write_bytes(resp)
        digest = hashlib.sha256(resp).hexdigest()
        return {
            "db": db_name,
            "name": name,
            "drive_file_id": file_id,
            "out": out_path,
            "bytes": len(resp),
            "sha256": digest,
        }

    # --------------------------------------------------------- manifest sync
    def _d1_endpoint(self) -> tuple[str, str]:
        """Resolve (bearer token, query URL) for the D1 manifest database.

        The token comes from the environment only; account/database IDs
        fall back env -> state.json -> the cold-store defaults.
        """
        token = os.environ.get("CLOUDFLARE_API_TOKEN")
        if not token:
            raise ColdStoreError(
                "CLOUDFLARE_API_TOKEN is not set — materialize it from "
                "Bitwarden into the environment first (it is never stored)"
            )
        state = self.load_state()
        account_id = (
            os.environ.get("COLD_STORE_CF_ACCOUNT_ID")
            or state.get("cf_account_id")
            or DEFAULT_CF_ACCOUNT_ID
        )
        database_id = (
            os.environ.get("COLD_STORE_CF_D1_DATABASE_ID")
            or state.get("cf_d1_database_id")
            or DEFAULT_CF_D1_DATABASE_ID
        )
        url = f"{self.d1_base}/accounts/{account_id}/d1/database/{database_id}/query"
        return token, url

    def _d1_select(self, sql: str) -> list[dict]:
        """Run one read query against D1 and return its result rows."""
        token, url = self._d1_endpoint()
        status, _h, resp = http_request(
            "POST",
            url,
            headers={
                "Authorization": f"Bearer {token}",
                "Content-Type": "application/json",
            },
            data=json.dumps({"sql": sql}).encode(),
        )
        if status != 200:
            raise ColdStoreError(
                f"D1 query failed (HTTP {status}): {d1_error_message(resp)}"
            )
        payload = json.loads(resp) if resp else {}
        if payload and not payload.get("success", True):
            errors = payload.get("errors") or [{}]
            raise ColdStoreError(
                f"D1 query rejected: {errors[0].get('message', 'unknown error')}"
            )
        results = (payload.get("result") or [{}])[0].get("results") or []
        return results

    @staticmethod
    def _table_shape(conn, table: str) -> tuple[list[str], list[str]]:
        rows = conn.execute(f"PRAGMA table_info({table})").fetchall()
        cols = [r["name"] for r in rows]
        pk = [
            r["name"]
            for r in sorted((r for r in rows if r["pk"]), key=lambda r: r["pk"])
        ]
        return cols, pk

    @staticmethod
    def _upsert_sql(table: str, cols: list[str], pk: list[str], nrows: int) -> str:
        one_group = "(" + ", ".join("?" for _ in cols) + ")"
        groups = ", ".join(one_group for _ in range(nrows))
        updates = [c for c in cols if c not in pk]
        if updates:
            action = "DO UPDATE SET " + ", ".join(f"{c}=excluded.{c}" for c in updates)
        else:
            action = "DO NOTHING"
        return (
            f"INSERT INTO {table} ({', '.join(cols)}) VALUES {groups} "
            f"ON CONFLICT({', '.join(pk)}) {action}"
        )

    def build_sync_statements(self) -> tuple[list[dict], dict]:
        """Read the local manifest and build batched D1 upsert statements.

        Returns (statements, per-table {"rows": n, "statements": m}). Each
        table's batch is sized by d1_sync_batch_rows(len(cols)) so a
        statement's bound parameters (rows x columns) stay within
        D1_SYNC_PARAM_BUDGET, under D1's per-query limit. Every statement
        is an idempotent upsert keyed by the table's primary key from
        schema.sql, so a re-sync is a handful of HTTP calls.
        """
        conn = self.db()
        statements: list[dict] = []
        counts: dict[str, dict] = {}
        for table in MANIFEST_TABLES:
            cols, pk = self._table_shape(conn, table)
            order = ", ".join(pk) if pk else cols[0]
            rows = conn.execute(
                f"SELECT {', '.join(cols)} FROM {table} ORDER BY {order}"
            ).fetchall()
            batch_rows = d1_sync_batch_rows(len(cols))
            table_statements = 0
            for i in range(0, len(rows), batch_rows):
                chunk = rows[i : i + batch_rows]
                params: list = []
                for row in chunk:
                    params.extend(row[c] for c in cols)
                statements.append(
                    {
                        "table": table,
                        "sql": self._upsert_sql(table, cols, pk, len(chunk)),
                        "params": params,
                        "rows": len(chunk),
                    }
                )
                table_statements += 1
            counts[table] = {"rows": len(rows), "statements": table_statements}
        conn.close()
        return statements, counts

    def sync_manifest(self, dry_run: bool = False) -> dict:
        statements, counts = self.build_sync_statements()
        total = {
            "tables": counts,
            "rows": sum(c["rows"] for c in counts.values()),
            "statements": len(statements),
            "dry_run": dry_run,
        }
        if dry_run:
            return total
        token, url = self._d1_endpoint()
        for stmt in statements:
            status, _h, resp = http_request(
                "POST",
                url,
                headers={
                    "Authorization": f"Bearer {token}",
                    "Content-Type": "application/json",
                },
                data=json.dumps(
                    {"sql": stmt["sql"], "params": stmt["params"]}
                ).encode(),
            )
            if status != 200:
                raise ColdStoreError(
                    f"D1 sync failed (HTTP {status}) on table {stmt['table']}: "
                    f"{d1_error_message(resp)} — statements already sent are "
                    "idempotent upserts, re-run to resume"
                )
            payload = json.loads(resp) if resp else {}
            if payload and not payload.get("success", True):
                errors = payload.get("errors") or [{}]
                raise ColdStoreError(
                    f"D1 sync rejected a {stmt['table']} statement: "
                    f"{errors[0].get('message', 'unknown error')}"
                )
        return total

    # ------------------------------------------------------------- backup-d1
    @staticmethod
    def _sql_literal(value) -> str:
        """Render one D1 JSON value as a deterministic SQL literal:
        NULL bare, numbers bare, strings single-quoted with '' escaping."""
        if value is None:
            return "NULL"
        if isinstance(value, bool):
            return "1" if value else "0"
        if isinstance(value, int):
            return str(value)
        if isinstance(value, float):
            return repr(value)
        return "'" + str(value).replace("'", "''") + "'"

    def render_d1_export(self) -> tuple[str, list[str], int]:
        """Export the whole D1 manifest database as deterministic SQL text.

        User tables are enumerated via sqlite_master (sqlite_% and _cf_%
        excluded), columns via pragma_table_info, rows in rowid order.
        Returns (sql_text, table_names, total_rows).
        """
        tables = [
            r["name"]
            for r in self._d1_select(
                "SELECT name FROM sqlite_master WHERE type='table'"
                " AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_cf_%'"
                " ORDER BY name"
            )
        ]
        lines = [
            "-- cold-store-manifest D1 export (via REST query API)",
            f"-- tables: {', '.join(tables)}",
        ]
        total = 0
        for t in tables:
            cols = [
                r["name"]
                for r in self._d1_select(f"SELECT name FROM pragma_table_info('{t}')")
            ]
            rows = self._d1_select(f'SELECT * FROM "{t}" ORDER BY rowid')
            for row in rows:
                vals = ", ".join(self._sql_literal(row[c]) for c in cols)
                lines.append(f'INSERT INTO "{t}" ({", ".join(cols)}) VALUES ({vals});')
            total += len(rows)
        return "\n".join(lines) + "\n", tables, total

    def backup_d1(self) -> dict:
        """Export the D1 cold-store-manifest database and file it as a
        d1 backup (DESIGN section 5), via the put-backup machinery."""
        text, tables, total = self.render_d1_export()
        staging = self.home / "staging"
        staging.mkdir(parents=True, exist_ok=True)
        path = staging / "cold-store-manifest-export.sql"
        path.write_text(text)
        result = self.put_backup("d1", str(path))
        result["tables"] = tables
        result["rows"] = total
        return result

    # ------------------------------------------------------------------- s3
    @staticmethod
    def _r2_credentials() -> tuple[str, str]:
        access = os.environ.get("R2_ACCESS_KEY_ID")
        secret = os.environ.get("R2_SECRET_ACCESS_KEY")
        if not access or not secret:
            raise ColdStoreError(
                "R2_ACCESS_KEY_ID / R2_SECRET_ACCESS_KEY are not set — "
                "materialize them from Bitwarden (cloudflare.r2-backups-*) "
                "into the environment first (they are never stored)"
            )
        return access, secret

    def _s3_signed_headers(self, method: str, path: str, query: str) -> dict:
        """SigV4 headers for one R2 request (payload is always empty:
        the client only lists, downloads and deletes)."""
        access, secret = self._r2_credentials()
        host = urllib.parse.urlparse(self.s3_base).netloc
        now = datetime.now(timezone.utc)
        amz_date = now.strftime("%Y%m%dT%H%M%SZ")
        date_stamp = now.strftime("%Y%m%d")
        payload_hash = hashlib.sha256(b"").hexdigest()
        signed = "host;x-amz-content-sha256;x-amz-date"
        canonical_headers = (
            f"host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n"
        )
        canonical_request = "\n".join(
            [method, path, query, canonical_headers, signed, payload_hash]
        )
        scope = f"{date_stamp}/{R2_SIGNING_REGION}/s3/aws4_request"
        string_to_sign = "\n".join(
            [
                "AWS4-HMAC-SHA256",
                amz_date,
                scope,
                hashlib.sha256(canonical_request.encode()).hexdigest(),
            ]
        )

        def _hmac(key, msg):
            return hmac.new(key, msg.encode(), hashlib.sha256).digest()

        k_signing = _hmac(
            _hmac(
                _hmac(_hmac(("AWS4" + secret).encode(), date_stamp), R2_SIGNING_REGION),
                "s3",
            ),
            "aws4_request",
        )
        signature = hmac.new(
            k_signing, string_to_sign.encode(), hashlib.sha256
        ).hexdigest()
        return {
            "Authorization": (
                f"AWS4-HMAC-SHA256 Credential={access}/{scope}, "
                f"SignedHeaders={signed}, Signature={signature}"
            ),
            "x-amz-content-sha256": payload_hash,
            "x-amz-date": amz_date,
        }

    def _s3_request(self, method, bucket, key=None, query=None, curl_fallback=True):
        """One signed S3 request; returns (status, headers, body).

        Transport failures — the response truncated mid-body
        (IncompleteRead), the peer dropping or resetting the
        connection (RemoteDisconnected / ConnectionResetError,
        including the URLError form http_request converts to
        ColdStoreError), or a garbled status line (BadStatusLine) —
        retry the WHOLE request, up to S3_TRANSPORT_ATTEMPTS attempts
        with a small backoff (self.s3_backoff_seconds x attempt,
        capped). LIST, GET and DELETE are all idempotent, so a blind
        retry is safe; the retry lives only in this S3 layer, never in
        http_request itself. The request is re-signed per attempt
        (SigV4 timestamps go stale; a fresh signature costs nothing).

        If every urllib attempt fails at the transport level and the
        method is GET, make exactly ONE curl attempt for the same URL
        (_s3_request_via_curl) before giving up: on a hostile-proxy
        night every urllib retry can truncate at a different offset
        (seen live 2026-10-28, when backup-supabase died at its LIST),
        while curl is immune. The curl response returns through the
        same (status, headers, body) shape, so callers' status
        handling (_s3_check) sees an HTTP error from curl exactly as
        it would from urllib. Non-GET methods get no fallback.
        s3_get_object passes curl_fallback=False: object downloads
        keep their dedicated byte-count-verified fallback
        (_s3_get_via_curl) instead of this unverified one.

        If every attempt fails, raise a ColdStoreError naming the
        operation and bucket instead of leaking a raw http.client
        traceback; when the curl fallback was attempted and also
        failed, the message says so.
        """
        path = "/" + urllib.parse.quote(bucket, safe="")
        if key is not None:
            path += "/" + urllib.parse.quote(key, safe="/")
        canonical_query = urllib.parse.urlencode(
            sorted((query or {}).items()), quote_via=urllib.parse.quote
        )
        url = self.s3_base + path
        if canonical_query:
            url += "?" + canonical_query
        what = f"{method} {bucket}" + (f"/{key}" if key is not None else "")
        problem: Exception | None = None
        for attempt in range(1, S3_TRANSPORT_ATTEMPTS + 1):
            headers = self._s3_signed_headers(method, path, canonical_query)
            try:
                return http_request(method, url, headers=headers)
            except (*S3_TRANSPORT_ERRORS, ColdStoreError) as e:
                problem = e
                if attempt < S3_TRANSPORT_ATTEMPTS:
                    delay = min(
                        self.s3_backoff_seconds * attempt,
                        S3_TRANSPORT_BACKOFF_CAP_SECONDS,
                    )
                    if delay > 0:
                        time.sleep(delay)
        if method == "GET" and curl_fallback:
            try:
                return self._s3_request_via_curl(url)
            except ColdStoreError as curl_problem:
                raise ColdStoreError(
                    f"S3 {what} failed after {S3_TRANSPORT_ATTEMPTS} attempts: "
                    f"transport error ({type(problem).__name__}: {problem}); "
                    f"curl fallback also failed: {curl_problem}"
                ) from None
        raise ColdStoreError(
            f"S3 {what} failed after {S3_TRANSPORT_ATTEMPTS} attempts: "
            f"transport error ({type(problem).__name__}: {problem})"
        )

    def _s3_request_via_curl(self, url: str):
        """One curl attempt at a GET-class _s3_request URL, in the
        same (status, headers, body) shape a urllib response has, so
        the caller's _s3_check treats an HTTP error status from curl
        exactly as it would from urllib. Headers are not captured
        (the GET-class callers — listings — read none); errors are
        the _curl_fetch_status ColdStoreErrors, already scrubbed."""
        access, secret = self._r2_credentials()
        status, body = self._curl_fetch_status(url, access, secret)
        return status, {}, body

    @staticmethod
    def _s3_check(status: int, body: bytes, what: str) -> None:
        if status >= 300:
            raise ColdStoreError(
                f"S3 {what} failed (HTTP {status}): {s3_error_message(body)}"
            )

    def s3_list_objects(self, bucket: str) -> list[dict]:
        """Every object in the bucket, following ListObjectsV2
        continuation tokens until the listing is complete.

        Progress guard: a truncated page must hand back a usable,
        advancing NextContinuationToken. A missing/empty token would
        re-request page 1 (the token only joins the query when truthy)
        and a repeated token re-requests the same page — both loop
        forever, appending duplicates. Raise instead, naming the
        bucket, so the caller sees a stuck endpoint, not a hang."""
        out: list[dict] = []
        token = None
        while True:
            query = {"list-type": "2"}
            if token:
                query["continuation-token"] = token
            status, _h, resp = self._s3_request("GET", bucket, query=query)
            self._s3_check(status, resp, f"list of {bucket}")
            page, truncated, next_token = _parse_s3_list_page(resp)
            out.extend(page)
            if not truncated:
                return out
            if not next_token or next_token == token:
                defect = (
                    "a missing or empty"
                    if not next_token
                    else "a non-advancing"
                )
                raise ColdStoreError(
                    f"list of {bucket}: truncated page returned {defect} "
                    "NextContinuationToken; aborting instead of "
                    "re-requesting the same page forever"
                )
            token = next_token

    def s3_get_object(
        self, bucket: str, key: str, expected_size: int | None = None
    ) -> bytes:
        """Download one object, verifying the received byte count
        against Content-Length. A truncated body — seen live through
        this sandbox's egress proxy, where urllib surfaces it as an
        http.client.IncompleteRead (an HTTPException) or a short read —
        retries the whole GET, up to S3_GET_ATTEMPTS attempts.

        If urllib ultimately cannot deliver the object whole, fall
        back to ONE curl attempt (_s3_get_via_curl): small responses
        truncate intermittently and retries absorb them, but
        multi-megabyte responses truncate systematically through this
        proxy (every attempt cut at the same byte count), while curl
        is immune — the same reason the Supabase backup tooling
        replays downloads with curl. `expected_size`, when the caller
        knows it (backup-supabase has it from the listing), is the
        byte count the curl body is verified against.
        """
        problem = "no attempt made"
        for _attempt in range(S3_GET_ATTEMPTS):
            try:
                # curl_fallback=False: downloads keep the dedicated
                # byte-count-verified curl fallback below rather than
                # _s3_request's unverified GET fallback.
                status, headers, resp = self._s3_request(
                    "GET", bucket, key=key, curl_fallback=False
                )
            except (http.client.HTTPException, ColdStoreError) as e:
                problem = str(e)
                continue
            self._s3_check(status, resp, f"GET {bucket}/{key}")
            expected = _header_value(headers, "Content-Length")
            if expected is not None and len(resp) != int(expected):
                problem = f"truncated body: got {len(resp)} of {expected} bytes"
                continue
            return resp
        try:
            return self._s3_get_via_curl(bucket, key, expected_size=expected_size)
        except ColdStoreError as e:
            raise ColdStoreError(
                f"S3 GET {bucket}/{key} failed after {S3_GET_ATTEMPTS} "
                f"urllib attempts ({problem}); {e}"
            ) from None

    def _s3_get_via_curl(
        self, bucket: str, key: str, expected_size: int | None = None
    ) -> bytes:
        """One curl download of one object, byte-count verified.

        The expected size is the caller's when given, else a urllib
        HEAD's Content-Length (HEAD has no body for the proxy to
        truncate). A body whose length cannot be established is not
        accepted: an unverifiable download is exactly what this path
        exists to avoid. Errors are detail-only — the caller
        (s3_get_object) names the operation, bucket and key.
        """
        access, secret = self._r2_credentials()
        if expected_size is None:
            try:
                status, headers, _body = self._s3_request("HEAD", bucket, key=key)
                if status == 200:
                    length = _header_value(headers, "Content-Length")
                    if length is not None:
                        expected_size = int(length)
            except ColdStoreError:
                pass  # no size from HEAD either; handled below
        if expected_size is None:
            raise ColdStoreError(
                "curl fallback could not determine the object's size "
                "(no caller size, no HEAD Content-Length); refusing "
                "an unverifiable download"
            )
        path = "/" + urllib.parse.quote(bucket, safe="")
        path += "/" + urllib.parse.quote(key, safe="/")
        body = self._curl_fetch(self.s3_base + path, access, secret)
        if len(body) != expected_size:
            raise ColdStoreError(
                f"curl fallback delivered {len(body)} of {expected_size} "
                "bytes (truncated)"
            )
        return body

    @staticmethod
    def _curl_config(access: str, secret: str) -> bytes:
        """The curl config carrying the credentials, fed to curl on
        stdin (--config -) by every curl call below: credentials
        never travel in argv — the local process list must not see
        them (the --user argv form was issue #983, the pattern
        #948/#970 removed from push-fleet-feed.sh) — and never on
        disk. They are scrubbed from every error the curl layer
        raises, and nothing in it is logged."""
        return (
            'user = "'
            + f"{access}:{secret}".replace("\\", "\\\\").replace('"', '\\"')
            + '"\n'
        ).encode()

    def _curl_spawn(self, argv: list, access: str, secret: str):
        """Run curl once (it does its own SigV4 signing via
        --aws-sigv4 and reads the proxy from the environment itself).
        A missing binary or a timeout is a ColdStoreError here; the
        exit code is the caller's to interpret."""
        try:
            return subprocess.run(
                argv,
                input=self._curl_config(access, secret),
                capture_output=True,
                timeout=S3_CURL_TIMEOUT_SECONDS,
            )
        except FileNotFoundError:
            raise ColdStoreError(
                f"curl binary not found (looked for {self.curl!r}) — "
                "the S3 curl fallback needs curl on PATH"
            ) from None
        except subprocess.TimeoutExpired:
            raise ColdStoreError(
                f"curl fallback timed out after {S3_CURL_TIMEOUT_SECONDS}s"
            ) from None

    @staticmethod
    def _curl_failure(proc, access: str, secret: str) -> ColdStoreError:
        detail = _scrub_secrets(
            proc.stderr.decode("utf-8", "replace").strip()[:200], access, secret
        )
        return ColdStoreError(
            f"curl fallback failed (exit {proc.returncode}): "
            f"{detail or 'no error output'}"
        )

    def _curl_fetch(self, url: str, access: str, secret: str) -> bytes:
        """GET `url` with curl and return the body; --fail makes any
        HTTP error status a curl failure, so a return is always a
        2xx body. Used by the object-download fallback, whose caller
        byte-count-verifies the result."""
        argv = [
            self.curl,
            "-sS",
            "--fail",
            "--aws-sigv4",
            "aws:amz:auto:s3",
            "--config",
            "-",
            url,
        ]
        proc = self._curl_spawn(argv, access, secret)
        if proc.returncode != 0:
            raise self._curl_failure(proc, access, secret)
        return proc.stdout

    # Appended to curl's stdout by --write-out so _curl_fetch_status
    # can split body from status unambiguously, whatever the body
    # itself ends with.
    _CURL_STATUS_TRAILER = "\n<<<coldstore-http-status:"

    def _curl_fetch_status(self, url: str, access: str, secret: str):
        """GET `url` with curl and return (status, body). No --fail:
        an HTTP error status is a normal result here, so the caller
        (_s3_request, via _s3_request_via_curl) can run it through
        the same _s3_check handling as a urllib response. A curl
        transport failure (nonzero exit — e.g. exit 18 when even curl
        gets a truncated transfer) raises like _curl_fetch."""
        argv = [
            self.curl,
            "-sS",
            "--aws-sigv4",
            "aws:amz:auto:s3",
            "--config",
            "-",
            "--write-out",
            self._CURL_STATUS_TRAILER + "%{http_code}>>>",
            url,
        ]
        proc = self._curl_spawn(argv, access, secret)
        if proc.returncode != 0:
            raise self._curl_failure(proc, access, secret)
        body, marker, tail = proc.stdout.rpartition(self._CURL_STATUS_TRAILER.encode())
        status_text = tail.decode("ascii", "replace")
        if not marker or not re.fullmatch(r"\d{3}>>>", status_text):
            raise ColdStoreError(
                "curl fallback returned an unparseable response "
                "(no HTTP status trailer)"
            )
        return int(status_text[:3]), body

    def s3_delete_object(self, bucket: str, key: str) -> None:
        status, _h, resp = self._s3_request("DELETE", bucket, key=key)
        self._s3_check(status, resp, f"DELETE {bucket}/{key}")

    # ---------------------------------------------------------- backup from R2
    def backup_supabase(self, bucket: str = DEFAULT_R2_BUCKET) -> dict:
        """File the newest Supabase backup object from R2 as a supabase
        backup (DESIGN section 5), via the put-backup machinery.

        'Newest' is by S3 LastModified among the bucket's backup
        objects; manifest.json is excluded — it is the backup system's
        own index, not a dump. The download is byte-count verified
        (s3_get_object). Filing is point-in-time, exactly like
        backup-d1: unchanged content is filed again, and the content
        sha256 is reported so the duplication is visible.
        """
        epoch = datetime.min.replace(tzinfo=timezone.utc)
        objects = [
            o
            for o in self.s3_list_objects(bucket)
            if o["key"].rsplit("/", 1)[-1] != "manifest.json"
        ]
        if not objects:
            raise ColdStoreError(f"no backup objects found in R2 bucket {bucket}")
        newest = max(objects, key=lambda o: (o["last_modified"] or epoch, o["key"]))
        data = self.s3_get_object(bucket, newest["key"], expected_size=newest["size"])
        staging = self.home / "staging"
        staging.mkdir(parents=True, exist_ok=True)
        path = staging / newest["key"].rsplit("/", 1)[-1]
        path.write_bytes(data)
        result = self.put_backup("supabase", str(path))
        result["sha256"] = hashlib.sha256(data).hexdigest()
        result["source_bucket"] = bucket
        result["source_key"] = newest["key"]
        return result

    # --------------------------------------------------------------- eviction
    def _object_cold_verified(self, conn, obj_hash: str):
        """(manifest state or None, member of a verified bundle?) —
        the two manifest facts every eviction decision rests on."""
        row = conn.execute(
            "SELECT state FROM objects WHERE hash = ?", (obj_hash,)
        ).fetchone()
        if row is None:
            return None, False
        member = conn.execute(
            "SELECT 1 AS x FROM members m JOIN bundles b"
            " ON b.bundle_id = m.bundle_id"
            " WHERE m.object_hash = ? AND b.state = 'verified' LIMIT 1",
            (obj_hash,),
        ).fetchone()
        return row["state"], member is not None

    def evict_scan(
        self,
        bucket: str = DEFAULT_R2_BUCKET,
        min_age_days: int = 14,
        keep_newest: int = 3,
    ) -> list[dict]:
        """One verdict per R2 object (manifest.json excluded): may it
        be evicted from R2 because the cold store already holds it?

        An object is 'evictable' only if ALL hold: (a) its content hash
        is known (content-addressed key), (b) the manifest has it
        'cold' in a verified bundle, (c) its LastModified is at least
        min_age_days old, (d) it is not among the keep_newest newest
        objects in the bucket. Anything else gets the first failing
        reason, checked in this order: hash-unknown / not-in-manifest /
        bundle-unverified / too-young / kept-newest. (Age is checked
        before position: a young object always has one of the newest
        LastModified stamps, so position-first would mask the age
        verdict entirely.) The scan never deletes.
        """
        epoch = datetime.min.replace(tzinfo=timezone.utc)
        objects = [
            o
            for o in self.s3_list_objects(bucket)
            if o["key"].rsplit("/", 1)[-1] != "manifest.json"
        ]
        newest_first = sorted(
            objects,
            key=lambda o: (o["last_modified"] or epoch, o["key"]),
            reverse=True,
        )
        protected = {o["key"] for o in newest_first[: max(keep_newest, 0)]}
        now = datetime.now(timezone.utc)
        min_age = timedelta(days=min_age_days)
        conn = self.db()
        scan = []
        for obj in newest_first:
            entry = {**obj, "hash": content_hash_from_key(obj["key"])}
            if entry["hash"] is None:
                entry["verdict"] = "hash-unknown"
            else:
                state, verified = self._object_cold_verified(conn, entry["hash"])
                if state != "cold":
                    entry["verdict"] = "not-in-manifest"
                    entry["detail"] = (
                        "no manifest row"
                        if state is None
                        else f"manifest state is {state!r}, not 'cold'"
                    )
                elif not verified:
                    entry["verdict"] = "bundle-unverified"
                elif (
                    obj["last_modified"] is None or now - obj["last_modified"] < min_age
                ):
                    entry["verdict"] = "too-young"
                elif obj["key"] in protected:
                    entry["verdict"] = "kept-newest"
                else:
                    entry["verdict"] = "evictable"
            scan.append(entry)
        conn.close()
        return scan

    def evict_apply(self, bucket: str, scan: list[dict]) -> list[dict]:
        """Delete a scan's evictable objects from R2, one by one.

        Each candidate is RE-VERIFIED against the manifest at deletion
        time (state 'cold' in a verified bundle) — the scan may be
        stale, and nothing is ever deleted on a stale verdict. After a
        successful DELETE, the object's manifest state becomes
        'evicted' and one ledger row is written (direction 'trash',
        kind 'evict', ref = R2 key). One object's failure is recorded
        in its outcome and does not abort the rest.
        """
        outcomes = []
        conn = self.db()
        for entry in scan:
            if entry["verdict"] != "evictable":
                continue
            key = entry["key"]
            try:
                state, verified = self._object_cold_verified(conn, entry["hash"])
                if state != "cold" or not verified:
                    outcomes.append(
                        {
                            "key": key,
                            "outcome": "skipped",
                            "detail": "re-verify failed at deletion time "
                            f"(state={state!r}, verified bundle={verified})",
                        }
                    )
                    continue
                self.s3_delete_object(bucket, key)
                with conn:
                    conn.execute(
                        "UPDATE objects SET state='evicted' WHERE hash=?",
                        (entry["hash"],),
                    )
                    self.ledger_add(conn, "trash", entry["size"], "evict", key)
                outcomes.append({"key": key, "outcome": "evicted", "detail": ""})
            except ColdStoreError as e:
                outcomes.append({"key": key, "outcome": "failed", "detail": str(e)})
        conn.close()
        return outcomes

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
        acct = self._upload_accounting(conn)
        cold_bytes = acct["cold_bytes"]
        backup_bytes = acct["backup_bytes"]
        up_24h = acct["uploads_24h_bytes"]
        down_24h = conn.execute(
            "SELECT coalesce(sum(bytes),0) n FROM transfer_ledger"
            " WHERE direction='download' AND ts >= datetime('now','-1 day')"
        ).fetchone()["n"]
        conn.close()
        hard_stop = acct["daily_upload_hard_stop_bytes"]
        budget = acct["project_budget_bytes"]
        return {
            "objects_total": objects_total,
            "objects_cold": objects_cold,
            "bundles_total": bundles_total,
            "bundles_verified": bundles_verified,
            "cold_bytes": cold_bytes,
            "backup_bytes": backup_bytes,
            "uploads_24h_bytes": up_24h,
            "downloads_24h_bytes": down_24h,
            "daily_upload_hard_stop_bytes": hard_stop,
            "hard_stop_ok": up_24h < hard_stop,
            "project_budget_bytes": budget,
            "within_budget": cold_bytes + backup_bytes <= budget if budget else True,
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


def _print_evict_scan(scan: list[dict]) -> None:
    counts: dict[str, int] = {}
    for entry in scan:
        counts[entry["verdict"]] = counts.get(entry["verdict"], 0) + 1
        stamp = (
            entry["last_modified"].strftime("%Y-%m-%dT%H:%M:%SZ")
            if entry["last_modified"]
            else "unknown-time"
        )
        line = (
            f"{entry['key']}: {entry['verdict']} "
            f"({_human(entry['size'])}, last modified {stamp})"
        )
        if entry.get("detail"):
            line += f" — {entry['detail']}"
        print(line)
    summary = ", ".join(f"{k} {v}" for k, v in sorted(counts.items()))
    print(f"summary: {len(scan)} object(s) scanned: {summary or 'none'}")


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
    p_put_backup = sub.add_parser(
        "put-backup", help="upload one DB dump, unbundled, into db-backups/<db>/"
    )
    p_put_backup.add_argument("db", choices=BACKUP_DBS)
    p_put_backup.add_argument("file")
    p_list_backups = sub.add_parser(
        "list-backups", help="list DB backups in Drive, newest first"
    )
    p_list_backups.add_argument("--db", choices=BACKUP_DBS, default=None)
    p_sync = sub.add_parser(
        "sync-manifest",
        help="push the local manifest to the D1 cold-store-manifest database (idempotent upserts)",
    )
    p_sync.add_argument(
        "--dry-run",
        action="store_true",
        help="print statement counts only; contact nothing",
    )
    p_prune = sub.add_parser(
        "prune-backups",
        help="enforce backup retention (30 daily / 12 weekly / 12 monthly, trash-not-delete); dry-run unless --apply",
    )
    p_prune.add_argument("--db", choices=BACKUP_DBS, default=None)
    p_prune.add_argument(
        "--apply",
        action="store_true",
        help="actually trash the prune candidates (default: print the plan only)",
    )
    p_get_backup = sub.add_parser(
        "get-backup", help="download one backup file in full by its exact name"
    )
    p_get_backup.add_argument("db", choices=BACKUP_DBS)
    p_get_backup.add_argument("name")
    p_get_backup.add_argument("out_path")
    sub.add_parser(
        "backup-d1",
        help="export the D1 cold-store-manifest database and file it as a d1 backup",
    )
    p_backup_sb = sub.add_parser(
        "backup-supabase",
        help="file the newest Supabase backup object from R2 into db-backups/supabase/",
    )
    p_backup_sb.add_argument("--bucket", default=DEFAULT_R2_BUCKET)
    p_evict_scan = sub.add_parser(
        "evict-scan",
        help="dry-run: verdict per R2 object — evictable only if cold in a verified bundle, old enough, and not among the newest kept (never deletes)",
    )
    p_evict_scan.add_argument("--bucket", default=DEFAULT_R2_BUCKET)
    p_evict_scan.add_argument("--min-age-days", type=int, default=14)
    p_evict_scan.add_argument("--keep-newest", type=int, default=3)
    p_evict = sub.add_parser(
        "evict",
        help="delete evictable objects from R2 (requires --apply; each is re-verified against the manifest at deletion time)",
    )
    p_evict.add_argument("--bucket", default=DEFAULT_R2_BUCKET)
    p_evict.add_argument("--min-age-days", type=int, default=14)
    p_evict.add_argument("--keep-newest", type=int, default=3)
    p_evict.add_argument(
        "--apply",
        action="store_true",
        help="actually delete (without it, prints the scan and exits non-zero)",
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
            keys = (
                ("root",) + SUBFOLDERS + tuple(f"db-backups/{db}" for db in BACKUP_DBS)
            )
            for key in keys:
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
                f"cold bytes in Drive: {_human(d['cold_bytes'])}"
                f" + backup bytes {_human(d['backup_bytes'])}"
                f" of budget {_human(d['project_budget_bytes'])}"
                f" ({'within' if d['within_budget'] else 'OVER'} budget)"
            )
            print(
                f"last 24h: uploads {_human(d['uploads_24h_bytes'])}, downloads {_human(d['downloads_24h_bytes'])}"
            )
            print(
                f"daily upload hard stop: {_human(d['daily_upload_hard_stop_bytes'])} — "
                f"{'OK' if d['hard_stop_ok'] else 'EXCEEDED'}"
            )
        elif args.cmd == "put-backup":
            result = store.put_backup(args.db, args.file)
            print(
                f"backup {result['name']}: {_human(result['bytes'])}, "
                f"uploaded to Drive as {result['drive_file_id']}"
            )
        elif args.cmd == "list-backups":
            rows = store.list_backups(args.db)
            if not rows:
                print("no backups found")
            for r in rows:
                print(
                    f"{r['created_time']}  {r['db']:<8} {_human(r['size']):>24}  "
                    f"{r['name']}  ({r['id']})"
                )
        elif args.cmd == "sync-manifest":
            result = store.sync_manifest(dry_run=args.dry_run)
            for table in MANIFEST_TABLES:
                c = result["tables"][table]
                print(f"{table}: {c['rows']} row(s) in {c['statements']} statement(s)")
            verb = "would send" if result["dry_run"] else "sent"
            print(
                f"total: {verb} {result['statements']} statement(s), "
                f"{result['rows']} row(s)"
            )
        elif args.cmd == "prune-backups":
            plans = store.prune_backups(args.db, apply=args.apply)
            for plan in plans:
                mode = "applied" if plan["applied"] else "dry-run"
                print(
                    f"{plan['db']}: keep {len(plan['keep'])}, "
                    f"prune {len(plan['prune'])}, "
                    f"skipped-unparseable {len(plan['unparseable'])} ({mode})"
                )
                for f in plan["keep"]:
                    print(f"  keep   {f['name']} [{', '.join(f['reasons'])}]")
                for f in plan["prune"]:
                    verb = "trashed" if plan["applied"] else "prune "
                    print(f"  {verb} {f['name']} [excess]")
                for f in plan["unparseable"]:
                    print(f"  skip   {f['name']} [unparseable]")
        elif args.cmd == "get-backup":
            result = store.get_backup(args.db, args.name, args.out_path)
            print(
                f"backup {result['db']}/{result['name']} -> {result['out']}: "
                f"{_human(result['bytes'])}, sha256 {result['sha256']}"
            )
        elif args.cmd == "backup-d1":
            result = store.backup_d1()
            print(
                f"backup {result['name']}: {_human(result['bytes'])}, "
                f"uploaded to Drive as {result['drive_file_id']} "
                f"({result['rows']} row(s) across {len(result['tables'])} table(s))"
            )
        elif args.cmd == "backup-supabase":
            result = store.backup_supabase(args.bucket)
            print(
                f"backup {result['name']}: {_human(result['bytes'])}, "
                f"sha256 {result['sha256']}, "
                f"uploaded to Drive as {result['drive_file_id']} "
                f"(from r2://{result['source_bucket']}/{result['source_key']})"
            )
        elif args.cmd == "evict-scan":
            scan = store.evict_scan(args.bucket, args.min_age_days, args.keep_newest)
            _print_evict_scan(scan)
        elif args.cmd == "evict":
            scan = store.evict_scan(args.bucket, args.min_age_days, args.keep_newest)
            _print_evict_scan(scan)
            if not args.apply:
                raise ColdStoreError(
                    "evict deletes objects from R2 — the scan above is a "
                    "dry run; re-run with --apply to delete the evictable "
                    "objects"
                )
            outcomes = store.evict_apply(args.bucket, scan)
            for o in outcomes:
                line = f"{o['outcome']}: {o['key']}"
                if o["detail"]:
                    line += f" — {o['detail']}"
                print(line)
            done = sum(1 for o in outcomes if o["outcome"] == "evicted")
            print(
                f"evict: {done} object(s) evicted, {len(outcomes) - done} not evicted"
            )
    except ColdStoreError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
