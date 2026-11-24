#!/usr/bin/env python3
"""Archer Slack collector: event-driven Socket Mode listener (#2137 PoC).

Replaces nothing yet. During the PoC this service runs IN PARALLEL with
the 15-minute watermark puller (archer-slack-review): it holds a Slack
Socket Mode WebSocket as the Archer app and appends every event for the
watched channels to the puller's SQLite store (table `collected_events`),
deduplicated on the event's natural identity, so the later fidelity
comparison (collector vs puller history) reads one store.

Deterministic collection only -- no LLM anywhere in this path. The app
token comes from the SLACK_APP_TOKEN environment variable (the wrapper
fetches it from Bitwarden Secrets Manager by name at service start);
it is never written to disk, logged, or printed.

Usage: SLACK_APP_TOKEN=xapp-... collector.py [--db PATH]
       [--channels C0CBYQR65LL,...]
"""

import argparse
import asyncio
import json
import os
import sqlite3
import sys
import time
import urllib.request
import urllib.error

API_BASE = "https://slack.com/api/"
DEFAULT_DB = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    "..",
    "puller",
    "state",
    "archer-slack.db",
)
DEFAULT_CHANNELS = "C0CBYQR65LL"  # #alignment, the PoC channel

SCHEMA = """
CREATE TABLE IF NOT EXISTS collected_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel TEXT NOT NULL,
    ts TEXT NOT NULL,
    event_type TEXT NOT NULL,
    subtype TEXT,
    user_id TEXT,
    thread_ts TEXT,
    reaction TEXT,
    payload TEXT NOT NULL,
    received_at TEXT NOT NULL,
    UNIQUE(channel, ts, event_type, user_id, reaction)
);
"""


def open_store(path):
    conn = sqlite3.connect(path)
    conn.execute(SCHEMA)
    conn.commit()
    return conn


def event_row(event):
    """Map one Events API event dict to a collected_events row (or None
    for events that carry no channel/ts identity worth storing)."""
    etype = event.get("type")
    if etype == "message":
        channel = event.get("channel")
        ts = event.get("ts")
        if not channel or not ts:
            return None
        return {
            "channel": channel,
            "ts": ts,
            "event_type": "message",
            "subtype": event.get("subtype"),
            # "" rather than None: SQLite treats NULLs as distinct in a
            # UNIQUE index, which would silently defeat dedupe.
            "user_id": event.get("user") or event.get("bot_id") or "",
            "thread_ts": event.get("thread_ts"),
            "reaction": "",
        }
    if etype == "reaction_added":
        item = event.get("item") or {}
        channel = item.get("channel")
        ts = item.get("ts")
        if not channel or not ts:
            return None
        return {
            "channel": channel,
            "ts": ts,
            "event_type": "reaction_added",
            "subtype": None,
            "user_id": event.get("user") or "",
            "thread_ts": None,
            "reaction": event.get("reaction"),
        }
    return None


def store_event(conn, event, watched, received_at=None):
    """Append an event if it is for a watched channel. Returns True when
    a row was inserted, False for ignored or duplicate events."""
    row = event_row(event)
    if row is None or row["channel"] not in watched:
        return False
    cur = conn.execute(
        "INSERT OR IGNORE INTO collected_events "
        "(channel, ts, event_type, subtype, user_id, thread_ts, reaction,"
        " payload, received_at) VALUES (?,?,?,?,?,?,?,?,?)",
        (
            row["channel"],
            row["ts"],
            row["event_type"],
            row["subtype"],
            row["user_id"],
            row["thread_ts"],
            row["reaction"],
            json.dumps(event, sort_keys=True),
            received_at or time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        ),
    )
    conn.commit()
    return cur.rowcount > 0


def open_socket_url(app_token):
    """POST apps.connections.open and return the WebSocket URL.

    Raises RuntimeError with Slack's error name on failure. The token is
    only ever placed in the Authorization header, never in a message."""
    req = urllib.request.Request(
        API_BASE + "apps.connections.open",
        data=b"",
        method="POST",
        headers={
            "Authorization": "Bearer " + app_token,
            "Content-Type": "application/x-www-form-urlencoded",
        },
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        data = json.loads(resp.read().decode("utf-8"))
    if not data.get("ok"):
        raise RuntimeError(
            "apps.connections.open failed: " + str(data.get("error", "unknown"))
        )
    return data["url"]


FATAL_ERRORS = {
    "invalid_auth",
    "not_allowed_token_type",
    "missing_scope",
    "token_revoked",
    "account_inactive",
}


async def run_forever(app_token, db_path, watched):
    import websockets  # deferred: unit tests import this module without it

    conn = open_store(db_path)
    backoff = 1
    while True:
        try:
            url = open_socket_url(app_token)
        except RuntimeError as exc:
            name = str(exc).rsplit(" ", 1)[-1]
            if name in FATAL_ERRORS:
                print(
                    f"collector: fatal Slack error ({name}); exiting so "
                    "the failure is loud instead of retried forever",
                    file=sys.stderr,
                    flush=True,
                )
                raise SystemExit(78)
            print(f"collector: connections.open retryable failure: {exc}", flush=True)
            await asyncio.sleep(backoff)
            backoff = min(backoff * 2, 60)
            continue
        try:
            async with websockets.connect(url, max_size=4 * 1024 * 1024) as ws:
                print("collector: socket connected", flush=True)
                backoff = 1
                async for raw in ws:
                    try:
                        env = json.loads(raw)
                    except ValueError:
                        continue
                    etype = env.get("type")
                    if etype == "hello":
                        print(
                            "collector: hello received "
                            f"(connections={env.get('num_connections')})",
                            flush=True,
                        )
                        continue
                    if etype == "disconnect":
                        print(
                            f"collector: disconnect requested "
                            f"({env.get('reason')}); reconnecting",
                            flush=True,
                        )
                        break
                    if etype == "events_api":
                        ack = {"envelope_id": env.get("envelope_id")}
                        await ws.send(json.dumps(ack))
                        payload = env.get("payload") or {}
                        event = payload.get("event") or {}
                        if store_event(conn, event, watched):
                            print(
                                f"collector: stored "
                                f"{event.get('type')} "
                                f"{event.get('channel') or (event.get('item') or {}).get('channel')}"
                                f" ts={(event.get('ts') or (event.get('item') or {}).get('ts'))}",
                                flush=True,
                            )
                    # slash_commands / interactive / block_actions: no PoC
                    # consumer; Socket Mode requires no ack for app
                    # behaviour we do not implement, and Archer is
                    # read-only in Slack by policy.
        except SystemExit:
            raise
        except Exception as exc:  # connection drop, DNS, TLS, ...
            print(
                f"collector: connection error ({exc!r}); reconnecting in {backoff}s",
                flush=True,
            )
            await asyncio.sleep(backoff)
            backoff = min(backoff * 2, 60)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--db", default=os.environ.get("ARCHER_SLACK_DB", DEFAULT_DB))
    ap.add_argument(
        "--channels", default=os.environ.get("ARCHER_WATCH_CHANNELS", DEFAULT_CHANNELS)
    )
    args = ap.parse_args(argv)
    token = os.environ.get("SLACK_APP_TOKEN")
    if not token:
        print("collector: SLACK_APP_TOKEN not set", file=sys.stderr)
        return 78
    watched = {c.strip() for c in args.channels.split(",") if c.strip()}
    print(f"collector: starting; db={args.db} channels={sorted(watched)}", flush=True)
    asyncio.run(run_forever(token, args.db, watched))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
