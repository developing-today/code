#!/usr/bin/env python3
"""Archer Slack puller: incremental channel review by watermark.

Pulls only the messages in each watched channel that are newer than the
last successful review (the watermark), including new replies in active
threads, so an agent run sees a small delta instead of re-reading whole
channels. State lives in a local SQLite file; the bot token is read from
an environment variable and never written to disk or printed.

Usage:
  SLACK_BOT_TOKEN=... python3 puller.py auth-test
  SLACK_BOT_TOKEN=... python3 puller.py list-channels
  SLACK_BOT_TOKEN=... python3 puller.py pull [--channel C123 ...] [--dry-run]
  SLACK_BOT_TOKEN=... python3 puller.py join --channel C123
  SLACK_BOT_TOKEN=... python3 puller.py post --channel C123 --text "hello"

`pull` prints one JSON object: {"messages": [...], "errors": [...],
"watermarks": {channel: ts}}. The watermark for a channel only advances
after its pull succeeds; a seen-set makes replays harmless
(at-least-once processing, deduped on (channel, ts)).
"""

import argparse
import json
import os
import sqlite3
import sys
import time
import urllib.parse
import urllib.request
import urllib.error

API_BASE = "https://slack.com/api/"
DEFAULT_DB = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                          "state", "archer-slack.db")
NOISE_SUBTYPES = {"channel_join", "channel_leave"}


class SlackAPIError(Exception):
    pass


class SlackClient:
    """Thin Slack Web API client (POST JSON, bearer token)."""

    def __init__(self, token):
        if not token:
            raise SlackAPIError("no bot token (set SLACK_BOT_TOKEN)")
        self._token = token

    def call(self, method, **params):
        # Form-encoding is Slack's canonical POST format. JSON bodies work
        # for most methods but conversations.replies rejects them with
        # invalid_arguments (verified live 2026-10-25), so use forms for all.
        form = {}
        for key, val in params.items():
            if val is None:
                continue
            if isinstance(val, bool):
                val = "true" if val else "false"
            form[key] = val
        body = urllib.parse.urlencode(form).encode("utf-8")
        req = urllib.request.Request(
            API_BASE + method, data=body, method="POST",
            headers={"Authorization": "Bearer " + self._token,
                     "Content-Type": "application/x-www-form-urlencoded"})
        data = None
        for attempt in range(3):
            try:
                with urllib.request.urlopen(req, timeout=30) as resp:
                    data = json.loads(resp.read().decode("utf-8"))
                break
            except urllib.error.HTTPError as e:
                if e.code == 429:
                    retry = e.headers.get("Retry-After", "?")
                    raise SlackAPIError(f"ratelimited on {method} "
                                        f"(retry after {retry}s)")
                raise SlackAPIError(f"HTTP {e.code} on {method}")
            except (ConnectionError, TimeoutError,
                    urllib.error.URLError) as e:
                # Transient transport failure (Slack intermittently closes
                # connections: http.client.RemoteDisconnected). Retry with
                # backoff — pulls are idempotent (watermark + seen-set).
                if attempt < 2:
                    time.sleep(2 * (attempt + 1))
                    continue
                raise SlackAPIError(
                    f"transport error on {method} after 3 attempts: {e}")
        if not data.get("ok", False):
            raise SlackAPIError(f"{method}: {data.get('error', 'unknown')}")
        return data


# ---------- state ----------

def open_state(path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    conn = sqlite3.connect(path)
    conn.executescript("""
        CREATE TABLE IF NOT EXISTS watermarks (
            channel_id TEXT PRIMARY KEY,
            watermark_ts TEXT NOT NULL,
            updated_at REAL NOT NULL
        );
        CREATE TABLE IF NOT EXISTS seen (
            channel_id TEXT NOT NULL,
            ts TEXT NOT NULL,
            PRIMARY KEY (channel_id, ts)
        );
        CREATE TABLE IF NOT EXISTS threads (
            channel_id TEXT NOT NULL,
            thread_ts TEXT NOT NULL,
            watermark_ts TEXT NOT NULL,
            PRIMARY KEY (channel_id, thread_ts)
        );
    """)
    return conn


def get_watermark(conn, channel):
    row = conn.execute(
        "SELECT watermark_ts FROM watermarks WHERE channel_id = ?",
        (channel,)).fetchone()
    return row[0] if row else "0"


def set_watermark(conn, channel, ts):
    conn.execute(
        "INSERT INTO watermarks VALUES (?, ?, ?) "
        "ON CONFLICT(channel_id) DO UPDATE SET watermark_ts = excluded.watermark_ts, "
        "updated_at = excluded.updated_at",
        (channel, ts, time.time()))


def is_seen(conn, channel, ts):
    return conn.execute(
        "SELECT 1 FROM seen WHERE channel_id = ? AND ts = ?",
        (channel, ts)).fetchone() is not None


def mark_seen(conn, channel, ts):
    conn.execute("INSERT OR IGNORE INTO seen VALUES (?, ?)", (channel, ts))


# ---------- pulling ----------

def _paged(client, method, items_key, **params):
    items, cursor = [], None
    while True:
        p = dict(params)
        if cursor:
            p["cursor"] = cursor
        data = client.call(method, **p)
        items.extend(data.get(items_key, []))
        cursor = (data.get("response_metadata") or {}).get("next_cursor")
        if not cursor:
            return items


def normalize(channel, msg, thread_ts=None):
    return {
        "channel": channel,
        "ts": msg.get("ts"),
        "thread_ts": thread_ts or msg.get("thread_ts"),
        "user": msg.get("user") or msg.get("bot_id"),
        "username": msg.get("username"),
        "text": msg.get("text", ""),
        "subtype": msg.get("subtype"),
        "reply_count": msg.get("reply_count", 0),
    }


def pull_channel(client, conn, channel, dry_run=False, overlap_seconds=0,
                 thread_lookback_seconds=7 * 24 * 3600, max_threads=25):
    """Return (new_messages, new_watermark). Raises SlackAPIError on failure."""
    wm = get_watermark(conn, channel)
    oldest = wm
    if overlap_seconds:
        oldest = str(max(0.0, float(wm) - overlap_seconds))
    # Slack's edge intermittently rejects known-good history calls with
    # invalid_arguments (verified 2026-10-29: two job runs failed while
    # the identical params succeeded on manual re-issue, three times).
    # One retry absorbs the transient; a second failure is real and
    # propagates. Safe because pulls are idempotent (watermark + seen-set).
    try:
        history = _paged(client, "conversations.history", "messages",
                         channel=channel, oldest=oldest, inclusive=False,
                         limit=200)
    except SlackAPIError as e:
        if "invalid_arguments" not in str(e):
            raise
        time.sleep(5)
        history = _paged(client, "conversations.history", "messages",
                         channel=channel, oldest=oldest, inclusive=False,
                         limit=200)
    fresh, max_ts, max_ts_str = [], float(wm), wm
    for msg in sorted(history, key=lambda m: float(m.get("ts", 0))):
        ts = msg.get("ts")
        if not ts or msg.get("subtype") in NOISE_SUBTYPES:
            if ts and float(ts) > max_ts:
                max_ts, max_ts_str = float(ts), ts
            continue
        if float(ts) > max_ts:
            max_ts, max_ts_str = float(ts), ts
        # Track every top-level message as a potential thread parent, not
        # just ones that already have replies: a reply can arrive later,
        # after the parent has scrolled behind the watermark, and the
        # sweep below is the only thing that would ever see it.
        conn.execute(
            "INSERT INTO threads VALUES (?, ?, ?) "
            "ON CONFLICT(channel_id, thread_ts) DO NOTHING",
            (channel, ts, ts))
        if is_seen(conn, channel, ts):
            continue
        mark_seen(conn, channel, ts)
        fresh.append(normalize(channel, msg))

    # Sweep active threads for new replies the top-level pull can't see.
    cutoff = time.time() - thread_lookback_seconds
    rows = conn.execute(
        "SELECT thread_ts, watermark_ts FROM threads WHERE channel_id = ? "
        "AND CAST(thread_ts AS REAL) > ? ORDER BY thread_ts DESC LIMIT ?",
        (channel, cutoff, max_threads)).fetchall()
    for thread_ts, thread_wm in rows:
        try:
            replies = _paged(client, "conversations.replies", "messages",
                              channel=channel, ts=thread_ts,
                              oldest=thread_wm, inclusive=False, limit=200)
        except SlackAPIError as e:
            # One bad thread must not poison the whole channel pull.
            # invalid_arguments is permanent (e.g. a thread_broadcast reply
            # tracked as a parent — Slack will never accept it), so drop the
            # thread from the sweep. Anything else (transport after retries)
            # keeps its row and is retried next cycle.
            print(f"puller: thread {thread_ts} in {channel} skipped: {e}",
                  file=sys.stderr)
            if "invalid_arguments" in str(e):
                conn.execute(
                    "DELETE FROM threads WHERE channel_id = ? "
                    "AND thread_ts = ?", (channel, thread_ts))
            continue
        new_thread_wm, new_thread_wm_str = float(thread_wm), thread_wm
        for rep in sorted(replies, key=lambda m: float(m.get("ts", 0))):
            rts = rep.get("ts")
            if not rts or rts == thread_ts:
                continue
            if float(rts) > new_thread_wm:
                new_thread_wm, new_thread_wm_str = float(rts), rts
            if is_seen(conn, channel, rts):
                continue
            mark_seen(conn, channel, rts)
            fresh.append(normalize(channel, rep, thread_ts=thread_ts))
        if new_thread_wm > float(thread_wm):
            conn.execute(
                "UPDATE threads SET watermark_ts = ? "
                "WHERE channel_id = ? AND thread_ts = ?",
                (new_thread_wm_str, channel, thread_ts))

    fresh.sort(key=lambda m: float(m["ts"]))
    if dry_run:
        conn.rollback()
        return fresh, wm
    if max_ts > float(wm):
        set_watermark(conn, channel, max_ts_str)
    conn.commit()
    return fresh, max_ts_str if max_ts > float(wm) else wm


def list_channels(client):
    chans = _paged(client, "conversations.list", "channels",
                   types="public_channel,private_channel",
                   exclude_archived=True, limit=200)
    return [c for c in chans if c.get("is_member", True)]


# ---------- CLI ----------

def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--db", default=DEFAULT_DB)
    ap.add_argument("--token-env", default="SLACK_BOT_TOKEN")
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("auth-test")
    sub.add_parser("list-channels")
    p_pull = sub.add_parser("pull")
    p_pull.add_argument("--channel", action="append", default=[])
    p_pull.add_argument("--dry-run", action="store_true")
    p_pull.add_argument("--overlap-seconds", type=int, default=0)
    p_join = sub.add_parser("join")
    p_join.add_argument("--channel", required=True)
    p_post = sub.add_parser("post")
    p_post.add_argument("--channel", required=True)
    p_post.add_argument("--text", required=True)
    p_post.add_argument("--thread-ts")
    args = ap.parse_args(argv)

    client = SlackClient(os.environ.get(args.token_env, ""))

    if args.cmd == "auth-test":
        data = client.call("auth.test")
        print(json.dumps({k: data.get(k) for k in
                          ("team", "team_id", "user", "user_id", "bot_id")},
                         indent=2))
        return 0
    if args.cmd == "list-channels":
        print(json.dumps(
            [{"id": c["id"], "name": c.get("name"),
              "is_private": c.get("is_private", False)}
             for c in list_channels(client)], indent=2))
        return 0
    if args.cmd == "join":
        client.call("conversations.join", channel=args.channel)
        print(json.dumps({"joined": args.channel}))
        return 0
    if args.cmd == "post":
        params = {"channel": args.channel, "text": args.text}
        if args.thread_ts:
            params["thread_ts"] = args.thread_ts
        data = client.call("chat.postMessage", **params)
        print(json.dumps({"posted": data.get("ts"),
                          "channel": data.get("channel")}))
        return 0
    if args.cmd == "pull":
        conn = open_state(args.db)
        channels = args.channel or [c["id"] for c in list_channels(client)]
        out = {"messages": [], "errors": [], "watermarks": {}}
        for channel in channels:
            try:
                fresh, wm = pull_channel(
                    client, conn, channel, dry_run=args.dry_run,
                    overlap_seconds=args.overlap_seconds)
                out["messages"].extend(fresh)
                out["watermarks"][channel] = wm
            except SlackAPIError as e:
                conn.rollback()
                out["errors"].append({"channel": channel, "error": str(e)})
        out["messages"].sort(key=lambda m: (m["channel"], float(m["ts"])))
        print(json.dumps(out, indent=2))
        return 1 if out["errors"] and not out["messages"] else 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
