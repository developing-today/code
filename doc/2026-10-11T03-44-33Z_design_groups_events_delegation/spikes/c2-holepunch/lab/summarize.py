"""Tabulates the dial summaries in logs/*-dial.txt as one markdown table. Run with `python3 -I`.

  python3 -I lab/summarize.py [LOGDIR]

Each row is one run, read from the dialer's summary block (src/main.rs, `dial`). Runs that
predate the `profile` line are shown with the default profile (QAD on, n0 on, no stand-in).
"""

import glob
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
logdir = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "logs")

# A path is "relay <url>", "direct <ip:port>", or "none".
PATH = r"(relay \S+|direct \S+|none)"


def grab(text, pattern, default="?"):
    m = re.search(pattern, text, re.M)
    return m.group(1) if m else default


def path_kind(value):
    return "relay" if value.startswith("relay") else value


def profile(text):
    m = re.search(r"^profile home_qad=(\w+) n0=(\w+)(?: standin=(\S+))?", text, re.M)
    if not m:
        return "default"
    home, n0, standin = m.groups()
    standin = "on" if standin and standin.startswith("Some") else "off"
    return f"home QAD {'on' if home == 'true' else 'off'}, n0 {'on' if n0 == 'true' else 'off'}, stand-in {standin}"


rows = []
for path in sorted(glob.glob(os.path.join(logdir, "*-dial.txt"))):
    name = os.path.basename(path)[: -len("-dial.txt")]
    text = open(path).read()
    if "--- summary" not in text:
        rows.append((name, "no summary, see log"))
        continue
    echoes = re.search(
        r"window_s=(\d+) echoes_ok=(\d+) echoes_failed=(\d+) echoes_on_direct=(\d+) echoes_on_relay=(\d+)",
        text,
    )
    window, ok, failed, on_direct, on_relay = echoes.groups()
    first_direct = grab(text, r"^first_direct_selected_ms=(\d+|none)")
    rows.append(
        (
            name,
            profile(text),
            grab(text, r"^\s*(\d+) ms online"),
            grab(text, r"^connected_ms=(\d+)"),
            path_kind(grab(text, rf"handshake_path={PATH}")),
            grab(text, r"^first_data_ms=(\d+)"),
            path_kind(grab(text, rf"first_data_path={PATH}")),
            first_direct,
            window,
            f"{on_direct}/{ok}",
            failed,
            path_kind(grab(text, rf"^selected_at_end={PATH}")),
            grab(text, r"^holepunch_attempts=(\d+)"),
        )
    )

print(
    "| run | profile | online ms | handshake ms | handshake path | first reply ms | first reply path"
    " | first direct ms | window s | echoes on direct / ok | failed | path at end | holepunch attempts |"
)
print("|" + "---|" * 13)
for row in rows:
    if len(row) == 2:
        print(f"| {row[0]} | {row[1]} |")
    else:
        print("| " + " | ".join(str(c) for c in row) + " |")
