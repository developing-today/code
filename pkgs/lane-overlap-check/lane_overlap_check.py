"""Same-lane overlap check for OpenChamber scheduled queue lanes.

A scheduled lane must not claim while an earlier occurrence of the same
lane is still running. The lane prompts used to answer that with
`openchamber session list --limit 250 --with-status`, which times out
at the control API's 4 s budget on a large server and cannot page, so
lanes stopped before claiming (code#11764).

This helper asks the OpenChamber server for only the lane's own
sessions (server-side title search, cursor paging), stops once the
listing is older than the window, and reads each candidate's status
with `openchamber session status`, which answers in well under a
second. It prints one JSON verdict:

  clear    no earlier occurrence of this lane is running; claim
  overlap  an earlier occurrence is busy, retrying or unknown; stop
  unknown  the check could not be completed; stop and report `error`

Exit status: 0 clear, 3 overlap, 4 unknown.
"""

import argparse
import json
import os
import subprocess
import sys
import time
import urllib.parse
import urllib.request

PREFIX = "Queue lane (scheduled) - "


def fetch_page(base, directory, search, cursor, limit, timeout):
    query = {"limit": str(limit), "search": search}
    if directory:
        query["directory"] = directory
    if cursor:
        query["cursor"] = cursor
    url = base.rstrip("/") + "/api/session?" + urllib.parse.urlencode(query)
    with urllib.request.urlopen(url, timeout=timeout) as resp:
        body = json.load(resp)
    if isinstance(body, list):
        return body, None
    return body.get("data") or [], (body.get("cursor") or {}).get("next")


def session_status(session_id, directory, timeout):
    cmd = ["openchamber", "session", "status", "--session", session_id]
    if directory:
        cmd += ["--dir", directory]
    cmd.append("--json")
    try:
        out = subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout, check=False
        )
        data = json.loads(out.stdout)
        return (data.get("sessionStatus") or {}).get("type") or "unknown"
    except (subprocess.SubprocessError, OSError, ValueError):
        return "unknown"


def directory_of(session):
    loc = session.get("location") or {}
    return loc.get("directory") or session.get("directory")


def list_lane_sessions(args, prefix, since_ms):
    """Every session titled with `prefix` updated since `since_ms`."""
    found = []
    cursor = None
    deadline = time.monotonic() + args.deadline
    for _ in range(args.max_pages):
        if time.monotonic() > deadline:
            raise TimeoutError("listing did not finish within the deadline")
        page, cursor = fetch_page(
            args.server, args.dir, prefix.strip(), cursor, args.page_size,
            args.http_timeout,
        )
        oldest = None
        for s in page:
            t = s.get("time") or {}
            updated = t.get("updated") or t.get("created") or 0
            oldest = updated if oldest is None else min(oldest, updated)
            if not (s.get("title") or "").startswith(prefix):
                continue
            if args.dir and directory_of(s) not in (None, args.dir):
                continue
            if updated >= since_ms or (t.get("created") or 0) >= since_ms:
                found.append(s)
        # Pages come newest-updated first: once a page reaches past the
        # window, nothing later can be a running occurrence.
        if not page or not cursor or (oldest is not None and oldest < since_ms):
            return found
    raise RuntimeError("listing did not reach the window in max pages")


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("lane", help="lane id, e.g. code-f")
    p.add_argument("--dir", default="/home/user/git/code")
    p.add_argument("--self", dest="self_id",
                   help="this occurrence's session id (default: the newest)")
    p.add_argument("--window-minutes", type=int, default=180)
    p.add_argument("--server", default=os.environ.get(
        "OPENCHAMBER_URL", "http://127.0.0.1:3000"))
    p.add_argument("--page-size", type=int, default=200)
    p.add_argument("--max-pages", type=int, default=20)
    p.add_argument("--http-timeout", type=float, default=10)
    p.add_argument("--status-timeout", type=float, default=15)
    p.add_argument("--deadline", type=float, default=60)
    args = p.parse_args(argv)

    prefix = PREFIX + args.lane + " "
    now_ms = int(time.time() * 1000)
    since_ms = now_ms - args.window_minutes * 60_000
    result = {"lane": args.lane, "prefix": prefix,
              "windowMinutes": args.window_minutes}
    try:
        sessions = list_lane_sessions(args, prefix, since_ms)
    except Exception as e:  # noqa: BLE001 - any failure means unknown
        result.update(verdict="unknown", error=f"{type(e).__name__}: {e}")
        print(json.dumps(result, indent=2))
        return 4

    sessions.sort(key=lambda s: (s.get("time") or {}).get("created") or 0)
    if args.self_id:
        me = next((s for s in sessions if s.get("id") == args.self_id), None)
    else:
        me = sessions[-1] if sessions else None
    if me is None:
        result.update(verdict="unknown",
                      error="this occurrence was not found in the listing")
        print(json.dumps(result, indent=2))
        return 4

    my_created = (me.get("time") or {}).get("created") or 0
    result["self"] = {"id": me["id"], "title": me.get("title"),
                      "created": my_created}
    earlier = []
    for s in sessions:
        created = (s.get("time") or {}).get("created") or 0
        if s["id"] == me["id"] or created > my_created:
            continue
        status = session_status(s["id"], directory_of(s) or args.dir,
                                args.status_timeout)
        earlier.append({"id": s["id"], "title": s.get("title"),
                        "created": created, "status": status})
    result["earlier"] = earlier
    running = [e for e in earlier if e["status"] != "idle"]
    result["verdict"] = "overlap" if running else "clear"
    print(json.dumps(result, indent=2))
    return 3 if running else 0


if __name__ == "__main__":
    sys.exit(main())
