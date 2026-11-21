#!/usr/bin/env python3
"""Fleet mailbox client for hosts whose `fleet` binary has no `bridge` verb yet.

The same agent verbs as `fleet bridge` (pkgs/fleet/crates/fleet-bridge), over
the message bridge's /fleet/<fleet token>/... routes, with the standard
library only. Use `fleet bridge` where the installed binary reports the
`bridge` capability (`fleet capabilities`); this script is the stopgap for
the sandbox install until ~/tools/fleet is reprovisioned.

    bridge-mailbox.py hello --agent muse-3 --groups muse
    bridge-mailbox.py inbox --agent muse-3 --groups muse --commands --take
    bridge-mailbox.py post --agent muse-3 --reply-to 42 "done: ..."
    bridge-mailbox.py post --agent muse-3 --to claude --instruction "..."
    bridge-mailbox.py ack --agent muse-3 --up-to 42

The fleet token is resolved by name, never passed on the command line:
`fleet secrets get message-bridge.fleet-token` (broker first, then bws),
or derived from `message-bridge.access-token` when that one is missing.
Agent and groups default to $FLEET_AGENT / $FLEET_AGENT_GROUPS, the URL to
$FLEET_BRIDGE_URL or https://bridge.security.cab. Output is the bridge's
JSON. Exit: 0 ok, 2 usage, 3 transport or token, 4 the bridge refused.
"""

import argparse
import base64
import hashlib
import hmac
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

TOKEN_NAME = "message-bridge.fleet-token"
OWNER_TOKEN_NAME = "message-bridge.access-token"
FLEET_TOKEN_LABEL = b"message-bridge/fleet-token/v1"
DEFAULT_URL = "https://bridge.security.cab"


def fail(code, msg, token=None):
    if token:
        msg = msg.replace(token, "<token>")
    print(f"bridge-mailbox: {msg}", file=sys.stderr)
    sys.exit(code)


def secret(name):
    """A secret's value by name through `fleet secrets get`, or None, plus the error text."""
    try:
        out = subprocess.run(
            ["fleet", "secrets", "get", name],
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as e:
        return None, f"could not run 'fleet secrets get {name}': {e}"
    value = out.stdout.strip()
    if out.returncode != 0 or not value:
        return None, out.stderr.strip()[:300]
    return value, ""


def fleet_token():
    """The fleet token, or derived from the owner token as the bridge does."""
    token, err = secret(TOKEN_NAME)
    if token:
        return token
    owner, _ = secret(OWNER_TOKEN_NAME)
    if owner:
        mac = hmac.new(owner.encode(), FLEET_TOKEN_LABEL, hashlib.sha256).digest()
        return base64.urlsafe_b64encode(mac).decode().rstrip("=")
    fail(
        3,
        f"fleet token '{TOKEN_NAME}' unresolved ({err}) and no '{OWNER_TOKEN_NAME}' to derive it from",
    )


def call(base, token, method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        f"{base}/fleet/{token}/{path}",
        data=data,
        method=method,
        headers={
            "accept": "application/json",
            "content-type": "application/json",
            "user-agent": "fleet-bridge-mailbox",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            return json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        text = e.read().decode(errors="replace")
        try:
            msg = json.loads(text).get("error") or text
        except ValueError:
            msg = (
                "not found (wrong token, URL, or a bridge without fleet mailboxes)"
                if e.code == 404
                else text
            )
        fail(4, f"the bridge answered {e.code}: {str(msg)[:300]}", token)
    except (urllib.error.URLError, OSError, ValueError) as e:
        fail(3, f"{method} to the bridge failed: {e}", token)


def text_arg(words):
    text = sys.stdin.read() if words == ["-"] else " ".join(words)
    if not text.strip():
        fail(2, "give the message text (or - to read it from stdin)")
    return text


def main():
    p = argparse.ArgumentParser(
        prog="bridge-mailbox.py", description="Fleet mailbox client (agent verbs)."
    )
    p.add_argument("--agent", default=os.environ.get("FLEET_AGENT"))
    sub = p.add_subparsers(dest="verb", required=True)
    h = sub.add_parser("hello")
    h.add_argument("--groups", default=os.environ.get("FLEET_AGENT_GROUPS"))
    i = sub.add_parser("inbox")
    i.add_argument("--groups", default=os.environ.get("FLEET_AGENT_GROUPS"))
    k = i.add_mutually_exclusive_group()
    k.add_argument("--commands", action="store_true")
    k.add_argument("--messages", action="store_true")
    i.add_argument("--all", action="store_true")
    i.add_argument("--take", action="store_true")
    i.add_argument("--since-id", type=int)
    i.add_argument("--limit", type=int)
    i.add_argument("--wait", type=int, default=0)
    i.add_argument("--interval", type=int, default=15)
    po = sub.add_parser("post")
    po.add_argument("--reply-to", type=int)
    po.add_argument("--to", action="append", default=[])
    po.add_argument("--instruction", action="store_true")
    po.add_argument("text", nargs="+")
    a = sub.add_parser("ack")
    a.add_argument("--up-to", type=int)
    a.add_argument("ids", nargs="*", type=int)
    # --agent may come before or after the verb.
    for sp in (h, i, po, a):
        sp.add_argument("--agent", dest="agent_after")
    args = p.parse_args()
    agent = (getattr(args, "agent_after", None) or args.agent or "").strip().lower()
    if not agent:
        fail(2, "name the agent: --agent NAME or $FLEET_AGENT")
    base = os.environ.get("FLEET_BRIDGE_URL", DEFAULT_URL).strip().rstrip("/")
    if not base.startswith("https://"):
        fail(2, "the bridge URL must be https")
    token = fleet_token()

    if args.verb == "hello":
        body = {"agent": agent}
        if args.groups is not None:
            body["groups"] = [g.strip() for g in args.groups.split(",") if g.strip()]
        out = call(base, token, "POST", "hello", body)
    elif args.verb == "inbox":
        q = {"agent": agent}
        if args.groups is not None:
            q["groups"] = args.groups
        if args.commands:
            q["kind"] = "command"
        if args.messages:
            q["kind"] = "message"
        if args.since_id is not None:
            q["since_id"] = str(args.since_id)
        if args.limit is not None:
            q["limit"] = str(args.limit)
        if args.all:
            q["unread_only"] = "false"
        if args.take:
            q["mark_read"] = "true"
        path = "inbox?" + urllib.parse.urlencode(q)
        deadline = time.monotonic() + max(0, args.wait)
        while True:
            out = call(base, token, "GET", path)
            if out.get("messages") or time.monotonic() >= deadline:
                break
            time.sleep(max(1, min(args.interval, deadline - time.monotonic())))
    elif args.verb == "post":
        body = {"agent": agent, "body": text_arg(args.text)}
        to = [t.strip() for x in args.to for t in x.split(",") if t.strip()]
        if args.instruction and not to:
            fail(2, "an instruction needs --to (central and replies take messages only)")
        if to:
            body["to"] = to
        if args.instruction:
            body["kind"] = "instruction"
        if args.reply_to is not None:
            body["reply_to"] = args.reply_to
        out = call(base, token, "POST", "send", body)
    else:
        if args.up_to is None and not args.ids:
            fail(2, "ack needs --up-to ID or one or more IDs")
        body = {"agent": agent}
        if args.up_to is not None:
            body["up_to_id"] = args.up_to
        if args.ids:
            body["ids"] = args.ids
        out = call(base, token, "POST", "ack", body)
    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    main()
