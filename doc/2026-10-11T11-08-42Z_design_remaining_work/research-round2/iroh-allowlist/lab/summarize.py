#!/usr/bin/env python3
"""Summarize N30 runs: one block per scenario, from logs/SCENARIO/run-*/{server,client}.log.

Usage (from the spike folder):
    python3 -I lab/summarize.py [LOG_ROOT] [SCENARIO ...]

Only the standard library is used. Every number printed here is computed from the log lines;
nothing is hand-entered.
"""

import glob
import math
import os
import re
import statistics
import sys
from datetime import datetime, timezone

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LOG_LINE = re.compile(r"^(\S+Z)\s+(\w+)\s+(.*)$")
ADDR_ADDED = re.compile(r"AddressAdded\(([^)]*)\)")
QNT = re.compile(r"updated local QNT addresses direct_addrs=\{([^}]*)\}")
REACH_RX = re.compile(r"got frame REACH_OUT round: \d+ local_addr: (\S+)")
ELAPSED = re.compile(r"elapsed_ms=([\d.]+)")


def parse_ts(s):
    return datetime.strptime(s, "%Y-%m-%dT%H:%M:%S.%fZ").replace(tzinfo=timezone.utc).timestamp()


def read_log(path):
    out = []
    if not os.path.exists(path):
        return out
    with open(path, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            m = LOG_LINE.match(line.rstrip("\n"))
            if m:
                out.append((parse_ts(m.group(1)), m.group(2), m.group(3)))
    return out


def ms(a, b):
    return (b - a) * 1000.0


def server_facts(lines):
    f = {"added_refuse_candidates": None}
    for t, _lvl, text in lines:
        if 'event="incoming"' in text and "incoming" not in f:
            f["incoming"] = t
            src = re.search(r"remote=Ip\(([^)]*)\)", text)
            f["udp_src"] = src.group(1) if src else None
            f["alpns"] = re.search(r"alpns=(\[[^\]]*\])", text).group(1)
        if "updated local QNT addresses" in text and "qnt" not in f:
            f["qnt"] = t
            f["qnt_addrs"] = QNT.search(text).group(1)
        if 'event="after_handshake"' in text and "hook" not in f:
            f["hook"] = t
            f["allowed"] = "allowed=true" in text
        for ev in (
            "refused_before_handshake",
            "handshake_refused",
            "application_has_connection",
            "stream_opened",
            "app_refusal",
            "connection_closed",
        ):
            if f'event="{ev}"' in text and ev not in f:
                f[ev] = t
        if "got frame REACH_OUT" in text:
            r = REACH_RX.search(text)
            f.setdefault("reach_out_rx", []).append((t, r.group(1) if r else "?"))
    return f


def client_facts(lines):
    f = {"addr_added": [], "hp_fail": []}
    for t, _lvl, text in lines:
        if 'event="connect_ok"' in text and "connect_ok" not in f:
            f["connect_ok"] = t
            f["connect_ok_ms"] = float(ELAPSED.search(text).group(1))
        if 'event="connect_err"' in text and "connect_err" not in f:
            f["connect_err"] = t
            f["connect_err_ms"] = float(ELAPSED.search(text).group(1))
            f["connect_err_msg"] = text.split("error=", 1)[1].split(" dbg=")[0]
        if 'event="closed_by_peer"' in text and "closed" not in f:
            f["closed"] = t
            f["closed_ms"] = float(ELAPSED.search(text).group(1))
            f["closed_msg"] = text.split("why=", 1)[1].split(" dbg=")[0]
        if 'event="closed_locally"' in text and "closed" not in f:
            f["closed_local"] = True
        if 'event="stream_reply"' in text:
            f["reply"] = text.split("reply=", 1)[1].strip()
        if 'event="stream_error"' in text:
            f["stream_error"] = text.split("error=", 1)[1].split(" dbg=")[0]
        for m in ADDR_ADDED.finditer(text):
            f["addr_added"].append((t, m.group(1)))
        if "failed to initiate NAT traversal" in text:
            f["hp_fail"].append(t)
        if "initiating NAT traversal round" in text:
            f.setdefault("reach_out_tx", t)
    return f


def wilson(k, n, z=1.96):
    if n == 0:
        return (0.0, 0.0)
    p = k / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return (max(0.0, c - h), min(1.0, c + h))


def pct(k, n):
    lo, hi = wilson(k, n)
    return f"{k}/{n} ({100.0 * k / n:.0f}%, 95% Wilson {100 * lo:.0f}-{100 * hi:.0f}%)" if n else f"0/0"


def stats(values):
    if not values:
        return "n/a"
    return f"median {statistics.median(values):.1f} ms (min {min(values):.1f}, max {max(values):.1f}, n={len(values)})"


def summarize(root, scenario):
    runs = sorted(glob.glob(os.path.join(root, scenario, "run-*")))
    if not runs:
        print(f"== {scenario}: no runs under {root}")
        return
    srv_all, cli_all = [], []
    port_of = {}
    for r in runs:
        ready = open(os.path.join(r, "ready.txt")).read() if os.path.exists(os.path.join(r, "ready.txt")) else ""
        port = re.search(r"port=(\d+)", ready)
        port_of[r] = port.group(1) if port else None
        srv_all.append(server_facts(read_log(os.path.join(r, "server.log"))))
        cli_all.append(client_facts(read_log(os.path.join(r, "client.log"))))

    n = len(runs)
    hook_ran = sum("hook" in s for s in srv_all)
    refused = sum("handshake_refused" in s for s in srv_all)
    handed = sum("application_has_connection" in s for s in srv_all)
    app_refused = sum("app_refusal" in s for s in srv_all)
    pre_refused = sum("refused_before_handshake" in s for s in srv_all)
    qnt_before_hook = sum(
        1 for s in srv_all if "qnt" in s and "hook" in s and s["qnt"] < s["hook"]
    )
    qnt_gap = [ms(s["qnt"], s["hook"]) for s in srv_all if "qnt" in s and "hook" in s]
    hook_to_refuse = [ms(s["hook"], s["handshake_refused"]) for s in srv_all if "hook" in s and "handshake_refused" in s]

    c_ok = sum("connect_ok" in c for c in cli_all)
    c_err = sum("connect_err" in c for c in cli_all)
    c_ok_ms = [c["connect_ok_ms"] for c in cli_all if "connect_ok_ms" in c]
    c_closed_ms = [c["closed_ms"] for c in cli_all if "closed_ms" in c]
    c_ok_to_close = [ms(c["connect_ok"], c["closed"]) for c in cli_all if "connect_ok" in c and "closed" in c]
    any_added = sum(1 for c in cli_all if c["addr_added"])
    new_added_runs = 0
    new_added_detail = {}
    for r, c in zip(runs, cli_all):
        port = port_of[r]
        new = sorted({a for _t, a in c["addr_added"] if port and not a.startswith(f"127.0.0.1:{port}")})
        if new:
            new_added_runs += 1
            new_added_detail[os.path.basename(r)] = new
    added_before_close = sum(
        1 for c in cli_all if c["addr_added"] and "closed" in c and c["addr_added"][0][0] < c["closed"]
    )
    hp_fail_runs = sum(1 for c in cli_all if c["hp_fail"])
    closed_msgs = {}
    for c in cli_all:
        if "closed_msg" in c:
            closed_msgs[c["closed_msg"]] = closed_msgs.get(c["closed_msg"], 0) + 1
    connect_err_msgs = {}
    for c in cli_all:
        if "connect_err_msg" in c:
            connect_err_msgs[c["connect_err_msg"]] = connect_err_msgs.get(c["connect_err_msg"], 0) + 1
    replies = {}
    for c in cli_all:
        if "reply" in c:
            replies[c["reply"]] = replies.get(c["reply"], 0) + 1
    stream_errs = {}
    for c in cli_all:
        if "stream_error" in c:
            stream_errs[c["stream_error"]] = stream_errs.get(c["stream_error"], 0) + 1
    # Cross-process check: both logs carry wall-clock stamps, so compare the client's connect()
    # return with the server's hook (the client finishes its handshake first).
    connect_before_hook = sum(
        1 for s, c in zip(srv_all, cli_all)
        if "hook" in s and "connect_ok" in c and c["connect_ok"] < s["hook"]
    )
    connect_err_ms = [c["connect_err_ms"] for c in cli_all if "connect_err_ms" in c]
    srv_allowed = [s.get("allowed") for s in srv_all if "hook" in s]
    reach_runs = sum(1 for s in srv_all if "reach_out_rx" in s)
    reach_before_hook = sum(1 for s in srv_all if "reach_out_rx" in s and "hook" in s and s["reach_out_rx"][0][0] < s["hook"])
    reach_before_refuse = sum(
        1
        for s in srv_all
        if "reach_out_rx" in s and ("handshake_refused" in s or "app_refusal" in s)
        and s["reach_out_rx"][0][0] < s.get("handshake_refused", s.get("app_refusal"))
    )
    reach_client_tx = sum(1 for c in cli_all if "reach_out_tx" in c)

    print(f"== {scenario} ({n} runs) ==")
    print(f"server  hook ran: {pct(hook_ran, n)}; allowed flag at hook: {sum(1 for a in srv_allowed if a)} allowed of {len(srv_allowed)}")
    print(f"server  refused in hook: {pct(refused, n)}; handed to application: {pct(handed, n)}; refused after stream: {pct(app_refused, n)}; refused before handshake: {pct(pre_refused, n)}")
    print(f"server  candidates queued before hook: {pct(qnt_before_hook, n)}; queue-to-hook gap {stats(qnt_gap)}")
    inc_qnt = [ms(s["incoming"], s["qnt"]) for s in srv_all if "qnt" in s and "incoming" in s]
    print(f"server  first Initial packet to candidates queued (server-side handshake) {stats(inc_qnt)}")
    print(f"server  hook-to-refusal gap {stats(hook_to_refuse)}")
    if reach_runs:
        print(f"server  REACH_OUT received (trace runs only): {pct(reach_runs, n)}; received before hook {pct(reach_before_hook, n)}; before refusal {pct(reach_before_refuse, n)}")
    print(f"client  connect() returned Ok: {pct(c_ok, n)}; connect() error: {pct(c_err, n)}")
    if connect_err_msgs:
        print(f"client  connect() error text: {connect_err_msgs}")
    print(f"client  connect() return time {stats(c_ok_ms)}")
    if connect_err_ms:
        print(f"client  connect() error time {stats(connect_err_ms)}")
    if any("hook" in s for s in srv_all):
        print(f"client  connect() returned before the server's hook ran: {pct(connect_before_hook, n)}")
    print(f"client  peer close observed time from start {stats(c_closed_ms)}; connect-return to close {stats(c_ok_to_close)}")
    print(f"client  close message(s): {closed_msgs if closed_msgs else 'none'}")
    if replies:
        print(f"client  stream reply(s): {replies}")
    if stream_errs:
        print(f"client  stream error(s): {stream_errs}")
    print(f"client  saw >=1 server AddressAdded before/at close: {pct(any_added, n)}; of those, AddressAdded before the close event: {added_before_close}")
    print(f"client  saw a server address NOT in the ticket (new information): {pct(new_added_runs, n)}")
    print(f"client  holepunch attempt failed ('connection is already closed' etc.) in: {pct(hp_fail_runs, n)}")
    if new_added_detail:
        first = next(iter(new_added_detail.items()))
        print(f"client  example new addresses ({first[0]}): {first[1]}")
    print()


def main():
    args = sys.argv[1:]
    root = os.path.join(HERE, "logs")
    if args and os.path.isdir(args[0]):
        root = args.pop(0)
    scenarios = args or sorted(
        d for d in os.listdir(root) if os.path.isdir(os.path.join(root, d))
    )
    for s in scenarios:
        summarize(root, s)


if __name__ == "__main__":
    main()
