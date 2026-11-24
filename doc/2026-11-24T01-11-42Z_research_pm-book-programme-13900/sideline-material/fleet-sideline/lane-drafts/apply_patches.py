#!/usr/bin/env python3
"""Surgical patches for the fleet book commander/bridge lane. Fails loud on any miss."""

import pathlib, sys

root = pathlib.Path.home() / "wt-fleet-cmdr-bridge-book" / "pkgs" / "fleet" / "book"


def patch(path, old, new, count=1):
    text = path.read_text()
    n = text.count(old)
    if n != count:
        print(f"MISS {path.name}: found {n} of expected {count} for: {old[:70]!r}")
        sys.exit(1)
    path.write_text(text.replace(old, new))
    print(f"OK {path.name}: {old[:50]!r} -> {new[:50]!r}")


# --- SUMMARY.md: register the commander chapter after the proxy concepts entry
summary = root / "SUMMARY.md"
patch(
    summary,
    "- [Proxy, sessions and recovery](concepts/proxy.md)\n",
    "- [Proxy, sessions and recovery](concepts/proxy.md)\n- [The Commander door: Desktop Commander over MCP](concepts/commander.md)\n",
)

# --- tutorials/bridge-messaging.md: stale help-shape + group-mailbox claims
tut = root / "tutorials" / "bridge-messaging.md"
patch(
    tut,
    "One help\nquirk: `fleet bridge` has a **single shared usage text** for the whole\ndomain — every per-verb `--help` prints the same block, so read the line\nfor the verb you are running.",
    "Every verb\nhas its own help: `fleet bridge <command> --help` prints that verb's\nusage section plus the shared defaults-and-exit-codes footer, and a bare\n`fleet bridge --help` prints the whole-domain usage text.",
)
patch(tut, "all from the shared usage:", "all from the verb's own `--help`:")
patch(
    tut,
    "- `--groups g1,g2`: also read group mailboxes for those groups.",
    "- `--groups g1,g2`: refresh this agent's registered groups (the same\n  groups `hello` records). There are no group mailboxes: a group send\n  lands as one row in each member agent's own mailbox at send time.",
)
patch(
    tut,
    "a group send lands in the\n  group mailbox, which you read by passing `--groups`)",
    "a group send lands as one\n  row in each registered member's own mailbox, so an agent that joins\n  the group later never sees it)",
)
patch(
    tut,
    "shared usage text is the reference.",
    "the verb's own `--help` text is the reference.",
)

# --- concepts/bridge.md: append the in-flight notes section
concepts = root / "concepts" / "bridge.md"
section = pathlib.Path("/tmp/bridge-notes-section.md").read_text()
text = concepts.read_text()
if "public wake signal" in text:
    print("MISS concepts/bridge.md: notes section already present")
    sys.exit(1)
concepts.write_text(text.rstrip("\n") + "\n" + section)
print("OK concepts/bridge.md: appended notes section")
print("ALL PATCHES APPLIED")
