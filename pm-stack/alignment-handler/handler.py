#!/usr/bin/env python3
"""Durable #alignment request handler (code#6651).

The handler consumes message events collected by the existing Archer
Socket Mode collector. It only considers the allowlisted #alignment
channel, owner mentions of Archer, and replies in threads where this
handler has already participated. Request state and replies are stored
in the collector's SQLite file so restarts do not lose work or duplicate
ACK/BLOCKED/RESULT messages.

`stage` runs a complete request through an offline fake sender. `watch`
is the long-running fleet process; it reads the collector store and posts
only to #alignment using the configured Archer bot token.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sqlite3
import sys
import time
import urllib.parse
import urllib.request
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Protocol


ALIGNMENT_CHANNEL_ID = "C0CBYQR65LL"
ALIGNMENT_OWNER_USER_ID = "U0CBX163G8Z"
DEFAULT_DB = os.environ.get(
    "ARCHER_SLACK_DB",
    os.path.expanduser("~/workspace/archer-slack/puller/state/archer-slack.db"),
)
API_BASE = "https://slack.com/api/"
FIELD_RE = re.compile(r"^\s*(id|owner|scope|evidence)\s*:\s*(.*?)\s*$", re.I)

SCHEMA = """
CREATE TABLE IF NOT EXISTS alignment_requests (
    request_id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL,
    source_ts TEXT NOT NULL,
    thread_ts TEXT NOT NULL,
    owner TEXT NOT NULL,
    scope TEXT NOT NULL,
    evidence TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('blocked', 'acknowledged', 'complete')),
    missing_json TEXT NOT NULL,
    result_text TEXT,
    result_evidence_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(channel_id, source_ts)
);
CREATE TABLE IF NOT EXISTS alignment_participating_threads (
    channel_id TEXT NOT NULL,
    thread_ts TEXT NOT NULL,
    request_id TEXT NOT NULL,
    PRIMARY KEY (channel_id, thread_ts, request_id)
);
CREATE TABLE IF NOT EXISTS alignment_processed_events (
    channel_id TEXT NOT NULL,
    message_ts TEXT NOT NULL,
    request_id TEXT,
    processed_at TEXT NOT NULL,
    PRIMARY KEY (channel_id, message_ts)
);
CREATE TABLE IF NOT EXISTS alignment_outbox (
    idempotency_key TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL,
    thread_ts TEXT NOT NULL,
    text TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    sent_ts TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS alignment_collector_cursor (
    channel_id TEXT PRIMARY KEY,
    event_id INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);
"""


class Sender(Protocol):
    def send(
        self, channel: str, thread_ts: str, text: str, idempotency_key: str
    ) -> dict[str, Any]: ...


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def _safe_slack_text(value: str) -> str:
    """Keep request-provided fields from turning replies into mentions."""
    text = " ".join(value.replace("\x00", "").split())
    text = text.replace("<@", "&lt;@").replace("<!", "&lt;!")
    for special in ("@channel", "@here", "@everyone"):
        text = text.replace(special, special.replace("@", "@ "))
    return text[:1200]


def _safe_request_id(value: str) -> str:
    """Keep caller IDs stable while preventing Slack markup/mentions in IDs."""
    safe = re.sub(r"[^A-Za-z0-9_-]+", "-", value).strip("-")[:80]
    return safe or "alignment-request"


def parse_fields(text: str) -> dict[str, str]:
    fields: dict[str, str] = {}
    for line in text.splitlines():
        match = FIELD_RE.match(line)
        if match:
            fields[match.group(1).lower()] = match.group(2).strip()
    return fields


def _outbox_key(channel: str, message_ts: str, phase: str) -> str:
    raw = f"alignment-handler\0{channel}\0{message_ts}\0{phase}".encode()
    return str(uuid.uuid5(uuid.NAMESPACE_URL, hashlib.sha256(raw).hexdigest()))


class AlignmentHandler:
    """Stateful, idempotent request handler backed by the collector DB."""

    def __init__(
        self,
        db_path: str,
        *,
        channel_id: str = ALIGNMENT_CHANNEL_ID,
        owner_user_id: str = ALIGNMENT_OWNER_USER_ID,
        bot_user_id: str,
    ):
        if channel_id != ALIGNMENT_CHANNEL_ID:
            raise ValueError("alignment handler may only watch #alignment")
        if not owner_user_id or not bot_user_id:
            raise ValueError("owner_user_id and bot_user_id are required")
        self.channel_id = channel_id
        self.owner_user_id = owner_user_id
        self.bot_user_id = bot_user_id
        if db_path != ":memory:":
            db_path = os.path.expanduser(db_path)
            Path(db_path).parent.mkdir(parents=True, exist_ok=True)
        self.conn = sqlite3.connect(db_path, timeout=30)
        self.conn.row_factory = sqlite3.Row
        self.conn.executescript(SCHEMA)
        self.conn.commit()

    def close(self) -> None:
        self.conn.close()

    def _thread_participates(self, thread_ts: str | None) -> bool:
        if not thread_ts:
            return False
        return self.conn.execute(
            "SELECT 1 FROM alignment_participating_threads "
            "WHERE channel_id=? AND thread_ts=? LIMIT 1",
            (self.channel_id, thread_ts),
        ).fetchone() is not None

    def _triggered(self, event: dict[str, Any]) -> bool:
        if event.get("type") != "message" or event.get("channel") != self.channel_id:
            return False
        if event.get("subtype") in {"bot_message", "message_changed", "message_deleted"}:
            return False
        user_id = event.get("user") or event.get("bot_id")
        if not user_id or user_id == self.bot_user_id:
            return False
        text = event.get("text") or ""
        owner_mention = (
            user_id == self.owner_user_id and f"<@{self.bot_user_id}>" in text
        )
        thread_reply = bool(event.get("thread_ts")) and self._thread_participates(
            event.get("thread_ts")
        )
        return owner_mention or thread_reply

    def handle_event(self, event: dict[str, Any], sender: Sender | None = None) -> str | None:
        """Record one Slack message and queue its ACK/BLOCKED reply.

        Replayed Slack events are harmless: the (channel, message ts) key
        is unique and an already processed event returns None.
        """
        if not self._triggered(event):
            return None
        message_ts = event.get("ts")
        if not isinstance(message_ts, str) or not message_ts:
            return None
        if self.conn.execute(
            "SELECT 1 FROM alignment_processed_events WHERE channel_id=? AND message_ts=?",
            (self.channel_id, message_ts),
        ).fetchone():
            return None

        fields = parse_fields(event.get("text") or "")
        user_id = str(event.get("user") or "")
        thread_ts = str(event.get("thread_ts") or message_ts)
        supplied_id = _safe_request_id(fields["id"]) if fields.get("id") else None
        if supplied_id:
            previous = self.conn.execute(
                "SELECT * FROM alignment_requests WHERE request_id=?", (supplied_id,)
            ).fetchone()
            if previous and previous["thread_ts"] == thread_ts and previous["state"] == "blocked":
                owner = _safe_slack_text(fields.get("owner") or previous["owner"] or user_id)
                scope = _safe_slack_text(fields.get("scope") or previous["scope"] or "")
                evidence = _safe_slack_text(fields.get("evidence") or previous["evidence"] or "")
                missing = [
                    name for name, value in (("scope", scope), ("evidence", evidence)) if not value
                ]
                state = "blocked" if missing else "acknowledged"
                now = utc_now()
                if missing:
                    reply = (
                        f"BLOCKED {supplied_id}\nOwner: {owner}\nScope: {scope or '(missing)'}\n"
                        f"Evidence: {evidence or '(missing)'}\n"
                        f"Missing: {', '.join(missing)}"
                    )
                else:
                    reply = (
                        f"ACK {supplied_id}\nOwner: {owner}\nScope: {scope}\nEvidence: {evidence}"
                    )
                key = _outbox_key(self.channel_id, message_ts, state)
                with self.conn:
                    self.conn.execute(
                        "UPDATE alignment_requests SET owner=?,scope=?,evidence=?,state=?,"
                        "missing_json=?,updated_at=? WHERE request_id=?",
                        (owner, scope, evidence, state, json.dumps(missing), now, supplied_id),
                    )
                    self.conn.execute(
                        "INSERT INTO alignment_processed_events VALUES (?,?,?,?)",
                        (self.channel_id, message_ts, supplied_id, now),
                    )
                    self.conn.execute(
                        "INSERT INTO alignment_outbox "
                        "(idempotency_key,channel_id,thread_ts,text,created_at) VALUES (?,?,?,?,?)",
                        (key, self.channel_id, thread_ts, reply, now),
                    )
                if sender is not None:
                    self.deliver_pending(sender)
                return supplied_id

        request_id = supplied_id or f"alignment-{message_ts.replace('.', '-') }"
        if self.conn.execute(
            "SELECT 1 FROM alignment_requests WHERE request_id=?", (request_id,)
        ).fetchone():
            request_id = f"{request_id}-{message_ts.replace('.', '-') }"

        owner = _safe_slack_text(fields.get("owner") or user_id)
        scope = _safe_slack_text(fields.get("scope") or "")
        evidence = _safe_slack_text(fields.get("evidence") or "")
        missing = [name for name, value in (("scope", scope), ("evidence", evidence)) if not value]
        state = "blocked" if missing else "acknowledged"
        now = utc_now()

        if missing:
            reply = (
                f"BLOCKED {request_id}\nOwner: {owner}\nScope: {scope or '(missing)'}\n"
                f"Evidence: {evidence or '(missing)'}\n"
                f"Missing: {', '.join(missing)}"
            )
        else:
            reply = (
                f"ACK {request_id}\nOwner: {owner}\nScope: {scope}\nEvidence: {evidence}"
            )
        key = _outbox_key(self.channel_id, message_ts, state)
        with self.conn:
            self.conn.execute(
                "INSERT INTO alignment_requests "
                "(request_id,channel_id,source_ts,thread_ts,owner,scope,evidence,state,"
                "missing_json,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?)",
                (request_id, self.channel_id, message_ts, thread_ts, owner, scope,
                 evidence, state, json.dumps(missing), now, now),
            )
            self.conn.execute(
                "INSERT INTO alignment_participating_threads VALUES (?,?,?)",
                (self.channel_id, thread_ts, request_id),
            )
            self.conn.execute(
                "INSERT INTO alignment_processed_events VALUES (?,?,?,?)",
                (self.channel_id, message_ts, request_id, now),
            )
            self.conn.execute(
                "INSERT INTO alignment_outbox "
                "(idempotency_key,channel_id,thread_ts,text,created_at) VALUES (?,?,?,?,?)",
                (key, self.channel_id, thread_ts, reply, now),
            )
        if sender is not None:
            self.deliver_pending(sender)
        return request_id

    def complete_request(
        self,
        request_id: str,
        *,
        result: str,
        evidence: list[str],
        sender: Sender | None = None,
    ) -> None:
        if not result.strip() or not evidence or any(not item.strip() for item in evidence):
            raise ValueError("RESULT requires a non-empty result and evidence link")
        row = self.conn.execute(
            "SELECT * FROM alignment_requests WHERE request_id=?", (request_id,)
        ).fetchone()
        if row is None:
            raise KeyError(f"unknown alignment request {request_id}")
        if row["state"] != "acknowledged":
            raise ValueError(f"request {request_id} is {row['state']}, not acknowledged")
        result_text = _safe_slack_text(result)
        safe_evidence = [_safe_slack_text(item) for item in evidence]
        text = (
            f"RESULT {request_id}\nOwner: {row['owner']}\nScope: {row['scope']}\n"
            f"{result_text}\nEvidence: " + " ".join(safe_evidence)
        )
        key = _outbox_key(row["channel_id"], row["source_ts"], "result")
        now = utc_now()
        with self.conn:
            self.conn.execute(
                "UPDATE alignment_requests SET state='complete', result_text=?, "
                "result_evidence_json=?, updated_at=? WHERE request_id=?",
                (result_text, json.dumps(safe_evidence), now, request_id),
            )
            self.conn.execute(
                "INSERT OR IGNORE INTO alignment_outbox "
                "(idempotency_key,channel_id,thread_ts,text,created_at) VALUES (?,?,?,?,?)",
                (key, row["channel_id"], row["thread_ts"], text, now),
            )
        if sender is not None:
            self.deliver_pending(sender)

    def deliver_pending(self, sender: Sender, limit: int = 100) -> dict[str, int]:
        """Deliver staged messages at least once; sender dedupes by key."""
        rows = self.conn.execute(
            "SELECT * FROM alignment_outbox WHERE sent_ts IS NULL "
            "ORDER BY created_at,idempotency_key LIMIT ?", (limit,)
        ).fetchall()
        sent = failed = 0
        for row in rows:
            try:
                response = sender.send(
                    row["channel_id"], row["thread_ts"], row["text"], row["idempotency_key"]
                )
                if not response.get("ok"):
                    raise RuntimeError(str(response.get("error", "sender rejected message")))
                with self.conn:
                    self.conn.execute(
                        "UPDATE alignment_outbox SET sent_ts=?, attempts=attempts+1, "
                        "last_error=NULL WHERE idempotency_key=?",
                        (str(response.get("ts") or utc_now()), row["idempotency_key"]),
                    )
                sent += 1
            except Exception as exc:
                with self.conn:
                    self.conn.execute(
                        "UPDATE alignment_outbox SET attempts=attempts+1, last_error=? "
                        "WHERE idempotency_key=?",
                        (str(exc)[:300], row["idempotency_key"]),
                    )
                failed += 1
        return {"sent": sent, "failed": failed}

    def poll_collector_store(self, sender: Sender, limit: int = 100) -> dict[str, int]:
        """Consume the existing collector's event rows for #alignment."""
        table = self.conn.execute(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='collected_events'"
        ).fetchone()
        if table is None:
            raise RuntimeError("collector store lacks the collected_events table")
        row = self.conn.execute(
            "SELECT event_id FROM alignment_collector_cursor WHERE channel_id=?",
            (self.channel_id,),
        ).fetchone()
        if row is None:
            # The collector store already contains historical #alignment
            # traffic. Establish a baseline instead of replying to old
            # messages when this handler is first installed.
            latest = self.conn.execute(
                "SELECT COALESCE(MAX(id),0) FROM collected_events WHERE channel=?",
                (self.channel_id,),
            ).fetchone()[0]
            after_id = int(latest)
            with self.conn:
                self.conn.execute(
                    "INSERT INTO alignment_collector_cursor(channel_id,event_id,updated_at) "
                    "VALUES(?,?,?)",
                    (self.channel_id, after_id, utc_now()),
                )
            return {"handled": 0, "ignored": 0, **self.deliver_pending(sender)}
        after_id = int(row[0])
        events = self.conn.execute(
            "SELECT id,payload FROM collected_events WHERE channel=? AND id>? "
            "ORDER BY id LIMIT ?", (self.channel_id, after_id, limit)
        ).fetchall()
        handled = ignored = 0
        for item in events:
            event = json.loads(item["payload"])
            request_id = self.handle_event(event, sender)
            handled += request_id is not None
            ignored += request_id is None
            with self.conn:
                self.conn.execute(
                    "INSERT INTO alignment_collector_cursor(channel_id,event_id,updated_at) "
                    "VALUES(?,?,?) ON CONFLICT(channel_id) DO UPDATE SET "
                    "event_id=excluded.event_id,updated_at=excluded.updated_at",
                    (self.channel_id, int(item["id"]), utc_now()),
                )
        delivered = self.deliver_pending(sender)
        return {"handled": handled, "ignored": ignored, **delivered}

    def get_request(self, request_id: str) -> dict[str, Any] | None:
        row = self.conn.execute(
            "SELECT * FROM alignment_requests WHERE request_id=?", (request_id,)
        ).fetchone()
        if row is None:
            return None
        result = dict(row)
        result["missing"] = json.loads(result.pop("missing_json"))
        raw_evidence = result.pop("result_evidence_json")
        result["result_evidence"] = json.loads(raw_evidence) if raw_evidence else []
        return result


class SlackApiSender:
    """Minimal Archer bot sender, constrained to the written #alignment scope."""

    def __init__(self, token: str):
        if not token:
            raise ValueError("SLACK_BOT_TOKEN is required")
        self.token = token

    def send(self, channel: str, thread_ts: str, text: str, idempotency_key: str) -> dict[str, Any]:
        if channel != ALIGNMENT_CHANNEL_ID:
            raise ValueError("alignment handler may only send to #alignment")
        client_msg_id = str(uuid.uuid5(uuid.NAMESPACE_URL, idempotency_key))
        body = urllib.parse.urlencode({
            "channel": channel,
            "thread_ts": thread_ts,
            "text": text,
            "client_msg_id": client_msg_id,
            "unfurl_links": "false",
            "unfurl_media": "false",
        }).encode()
        req = urllib.request.Request(
            API_BASE + "chat.postMessage",
            data=body,
            method="POST",
            headers={
                "Authorization": "Bearer " + self.token,
                "Content-Type": "application/x-www-form-urlencoded",
            },
        )
        with urllib.request.urlopen(req, timeout=10) as response:
            payload = json.loads(response.read().decode("utf-8"))
        if not payload.get("ok"):
            return {"ok": False, "error": payload.get("error", "unknown")}
        return {"ok": True, "ts": payload.get("ts")}


class StageSender:
    """Offline sender used by `stage` and the staged end-to-end tests."""

    def __init__(self):
        self.messages: dict[str, tuple[str, str, str, str]] = {}

    def send(self, channel: str, thread_ts: str, text: str, idempotency_key: str) -> dict[str, Any]:
        if channel != ALIGNMENT_CHANNEL_ID:
            return {"ok": False, "error": "channel is outside staged #alignment scope"}
        self.messages.setdefault(idempotency_key, (channel, thread_ts, text, idempotency_key))
        return {"ok": True, "ts": f"staged-{len(self.messages)}"}


def run_stage() -> dict[str, Any]:
    """Run one complete request without Slack, credentials, or network I/O."""
    sender = StageSender()
    handler = AlignmentHandler(
        ":memory:", channel_id=ALIGNMENT_CHANNEL_ID,
        owner_user_id=ALIGNMENT_OWNER_USER_ID, bot_user_id="U0CBTJGQQSZ",
    )
    event = {
        "type": "message", "channel": ALIGNMENT_CHANNEL_ID, "ts": "stage.001",
        "user": ALIGNMENT_OWNER_USER_ID,
        "text": "<@U0CBTJGQQSZ>\nID: stage-6651\nOwner: @drew\n"
                "Scope: verify the staged handler\nEvidence: fixture://alignment/6651",
    }
    request_id = handler.handle_event(event, sender)
    handler.complete_request(
        request_id,
        result="The staged request completed through the handler contract.",
        evidence=["fixture://alignment/6651/result"],
        sender=sender,
    )
    item = handler.get_request(request_id)
    messages = [entry[2] for entry in sender.messages.values()]
    handler.close()
    return {"request": item, "messages": messages, "external_send": False}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("stage", help="run one offline staged ACK/RESULT request", allow_abbrev=False)
    watch = sub.add_parser("watch", help="consume the collector SQLite store", allow_abbrev=False)
    watch.add_argument("--db", default=DEFAULT_DB)
    watch.add_argument("--bot-user-id", default=os.environ.get("ALIGNMENT_HANDLER_BOT_USER_ID"))
    watch.add_argument("--poll-seconds", type=float, default=5.0)
    result = sub.add_parser("result", help="post a completed request result in its thread", allow_abbrev=False)
    result.add_argument("--db", default=DEFAULT_DB)
    result.add_argument("--request-id", required=True)
    result.add_argument("--text", required=True)
    result.add_argument("--evidence", action="append", required=True)
    args = parser.parse_args(argv)

    if args.command == "stage":
        print(json.dumps(run_stage(), sort_keys=True))
        return 0
    token = os.environ.get("SLACK_BOT_TOKEN")
    if not token:
        print("alignment-handler: SLACK_BOT_TOKEN is required", file=sys.stderr)
        return 78
    sender = SlackApiSender(token)
    if args.command == "result":
        store = AlignmentHandler(
            args.db, owner_user_id=ALIGNMENT_OWNER_USER_ID,
            bot_user_id=os.environ.get("ALIGNMENT_HANDLER_BOT_USER_ID", "U0CBTJGQQSZ"),
        )
        try:
            store.complete_request(
                args.request_id, result=args.text, evidence=args.evidence, sender=sender
            )
            print(json.dumps({"result": "queued", "request_id": args.request_id}))
            return 0
        finally:
            store.close()
    if args.poll_seconds <= 0:
        parser.error("--poll-seconds must be greater than zero")
    if not args.bot_user_id:
        print("alignment-handler: --bot-user-id or ALIGNMENT_HANDLER_BOT_USER_ID is required", file=sys.stderr)
        return 78
    store = AlignmentHandler(
        args.db, owner_user_id=ALIGNMENT_OWNER_USER_ID, bot_user_id=args.bot_user_id
    )
    try:
        while True:
            summary = store.poll_collector_store(sender)
            print(json.dumps(summary, sort_keys=True), flush=True)
            time.sleep(args.poll_seconds)
    finally:
        store.close()


if __name__ == "__main__":
    raise SystemExit(main())
