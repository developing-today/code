#!/usr/bin/env python3
"""Offline tests for puller.py using a scripted fake Slack client."""

import os
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import puller

BASE = int(time.time()) - 100_000


def ts(n):
    return f"{BASE}.{n:06d}"


class FakeClient:
    def __init__(self):
        self.history = {"C1": [
            {"ts": ts(1), "user": "U1", "text": "first"},
            {"ts": ts(2), "user": "U2", "text": "parent",
             "thread_ts": ts(2), "reply_count": 1},
            {"ts": ts(3), "user": "U1", "text": "third",
             "subtype": "channel_join"},
        ]}
        self.replies = {ts(2): [
            {"ts": ts(2), "user": "U2", "text": "parent",
             "thread_ts": ts(2)},
            {"ts": ts(5), "user": "U1", "text": "a reply",
             "thread_ts": ts(2)},
        ]}

    def call(self, method, **params):
        if method == "conversations.history":
            oldest = float(params.get("oldest", "0"))
            msgs = [m for m in self.history[params["channel"]]
                    if float(m["ts"]) > oldest]
            return {"ok": True, "messages": msgs, "response_metadata": {}}
        if method == "conversations.replies":
            oldest = float(params.get("oldest", "0"))
            if params["ts"] in self.replies:
                msgs = [m for m in self.replies[params["ts"]]
                        if float(m["ts"]) > oldest or m["ts"] == params["ts"]]
            else:
                # No thread: the real API returns just the parent message.
                msgs = [m for chan in self.history.values() for m in chan
                        if m["ts"] == params["ts"]]
            return {"ok": True, "messages": msgs, "response_metadata": {}}
        raise AssertionError("unexpected method " + method)


def main():
    tmp = tempfile.mkdtemp()
    conn = puller.open_state(os.path.join(tmp, "test.db"))
    client = FakeClient()

    fresh, wm = puller.pull_channel(client, conn, "C1")
    texts = [m["text"] for m in fresh]
    assert texts == ["first", "parent", "a reply"], texts
    assert wm == ts(3), wm  # join noise still advances the watermark

    # Second pull: nothing new.
    fresh2, wm2 = puller.pull_channel(client, conn, "C1")
    assert fresh2 == [], fresh2
    assert wm2 == wm, (wm2, wm)

    # A new top-level message and a new thread reply appear.
    client.history["C1"].append(
        {"ts": ts(10), "user": "U2", "text": "fourth"})
    client.replies[ts(2)].append(
        {"ts": ts(11), "user": "U2", "text": "second reply",
         "thread_ts": ts(2)})
    fresh3, wm3 = puller.pull_channel(client, conn, "C1")
    texts3 = [m["text"] for m in fresh3]
    assert texts3 == ["fourth", "second reply"], texts3
    assert wm3 == ts(10), wm3

    # Dry run reports but does not persist: the same message comes back.
    client.history["C1"].append(
        {"ts": ts(20), "user": "U1", "text": "fifth"})
    fresh4, _ = puller.pull_channel(client, conn, "C1", dry_run=True)
    assert [m["text"] for m in fresh4] == ["fifth"], fresh4
    fresh5, _ = puller.pull_channel(client, conn, "C1")
    assert [m["text"] for m in fresh5] == ["fifth"], fresh5

    print("all puller tests passed")


if __name__ == "__main__":
    main()
