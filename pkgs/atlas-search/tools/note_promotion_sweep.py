#!/usr/bin/env python3
"""note-promotion-sweep — pointer-stub sweep for note promotions (#8206).

Implements S7 item 4 of the agent-note placement design
(doc/2026-11-10T20-00-00Z_design_agent_note_placement.md, issue #6022),
under its S6 migration rule: **move on next touch, never big-bang**.

When a note is promoted to a new home (per the design S4 paths — usually
a grandfathered research/workspace note promoted to zettel on its next
citation or revision), one run of this tool performs the whole move:

  1. the note's bytes are written to the new home, unchanged;
  2. the old -> new mapping is recorded in the submission DB
     (table `note_promotions` — the Turso submission DB is SQLite-shaped;
     a local file path stands in for its URI in tests and dry runs);
  3. the old location is replaced with a pointer stub naming the new
     home's identity, so old links keep resolving.

Safety rules (the sweep never loses a note):
  * the new home is written and verified (content hash) BEFORE the old
    file becomes a stub;
  * an existing new home with different content is a refusal, never a
    clobber;
  * re-running over a completed promotion is a no-op receipt
    (`already_promoted`), and a pointer stub is never itself promoted;
  * the DB record is keyed by the old path and written in the same run,
    before the stub replaces the original.

Usage:
  note_promotion_sweep.py promote --old PATH --new PATH --db DB
      [--new-home-id ID] [--now ISO8601]
  note_promotion_sweep.py sweep --map promotions.json --db DB
      (promotions.json: [{"old": ..., "new": ..., "new_home_id": ...}])

Exit codes: 0 ok (incl. already_promoted), 2 refusal, 3 usage/IO error.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sqlite3
import sys
from datetime import datetime, timezone
from pathlib import Path

STUB_MARKER = "pointer-stub: true"

SCHEMA = """
CREATE TABLE IF NOT EXISTS note_promotions (
  old_path      TEXT PRIMARY KEY,
  new_path      TEXT NOT NULL,
  new_home_id   TEXT NOT NULL,
  content_sha256 TEXT NOT NULL,
  promoted_at   TEXT NOT NULL
);
"""

STUB_TEMPLATE = """\
---
{marker}
promoted-to: {new_home_id}
promoted-path: {new_path}
content-sha256: {sha}
promoted-at: {now}
design: doc/2026-11-10T20-00-00Z_design_agent_note_placement.md (S6: move on next touch)
---

# Pointer stub — this note was promoted

This note moved to its new home: **{new_home_id}** (`{new_path}`).

The content is unchanged (sha256 `{sha}`) and the move is recorded in
the submission DB (`note_promotions`: old -> new). Follow the new home;
do not edit this stub.
"""


class Refusal(Exception):
    """A promotion the sweep will not perform (exit 2)."""


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def is_stub(text: str) -> bool:
    return STUB_MARKER in text.split("---", 2)[1] if text.startswith("---") else False


def record_promotion(
    db_path: Path, old: str, new: str, home_id: str, sha: str, now: str
) -> None:
    db_path.parent.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(db_path) as conn:
        conn.execute(SCHEMA)
        conn.execute(
            "INSERT INTO note_promotions"
            " (old_path, new_path, new_home_id, content_sha256, promoted_at)"
            " VALUES (?, ?, ?, ?, ?)"
            " ON CONFLICT(old_path) DO NOTHING",
            (old, new, home_id, sha, now),
        )


def lookup_promotion(db_path: Path, old: str) -> dict | None:
    if not db_path.exists():
        return None
    with sqlite3.connect(db_path) as conn:
        conn.execute(SCHEMA)
        row = conn.execute(
            "SELECT new_path, new_home_id, content_sha256, promoted_at"
            " FROM note_promotions WHERE old_path = ?",
            (old,),
        ).fetchone()
    if row is None:
        return None
    return {
        "new_path": row[0],
        "new_home_id": row[1],
        "content_sha256": row[2],
        "promoted_at": row[3],
    }


def promote(
    old: Path,
    new: Path,
    db_path: Path,
    new_home_id: str | None = None,
    now: str | None = None,
) -> dict:
    """Perform one promotion; returns the receipt dict."""
    now = now or datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    home_id = new_home_id or str(new)
    old_s, new_s = str(old), str(new)

    if old.resolve() == new.resolve():
        raise Refusal(f"old and new are the same path: {old_s}")
    if not old.is_file():
        raise Refusal(f"old note does not exist: {old_s}")

    original = old.read_bytes()
    if is_stub(original.decode("utf-8", errors="replace")):
        prior = lookup_promotion(db_path, old_s)
        if prior and prior["new_path"] == new_s:
            return {"status": "already_promoted", "old": old_s, **prior}
        raise Refusal(f"old location is already a pointer stub: {old_s}")

    sha = sha256_hex(original)

    if new.exists():
        if not new.is_file() or sha256_hex(new.read_bytes()) != sha:
            raise Refusal(f"new home already exists with different content: {new_s}")
    else:
        new.parent.mkdir(parents=True, exist_ok=True)
        new.write_bytes(original)
        if sha256_hex(new.read_bytes()) != sha:
            raise Refusal(f"verification failed after writing new home: {new_s}")

    record_promotion(db_path, old_s, new_s, home_id, sha, now)

    stub = STUB_TEMPLATE.format(
        marker=STUB_MARKER, new_home_id=home_id, new_path=new_s, sha=sha, now=now
    )
    old.write_text(stub, encoding="utf-8")
    return {
        "status": "promoted",
        "old": old_s,
        "new_path": new_s,
        "new_home_id": home_id,
        "content_sha256": sha,
        "promoted_at": now,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="note-promotion-sweep", description=__doc__)
    sub = parser.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("promote", help="promote one note")
    p.add_argument("--old", required=True)
    p.add_argument("--new", required=True)
    p.add_argument("--db", required=True)
    p.add_argument("--new-home-id")
    p.add_argument("--now")

    s = sub.add_parser("sweep", help="promote every entry of a JSON map")
    s.add_argument("--map", required=True)
    s.add_argument("--db", required=True)
    s.add_argument("--now")

    args = parser.parse_args(argv)
    try:
        if args.cmd == "promote":
            receipt = promote(
                Path(args.old),
                Path(args.new),
                Path(args.db),
                args.new_home_id,
                args.now,
            )
            print(json.dumps(receipt, indent=2))
            return 0
        entries = json.loads(Path(args.map).read_text(encoding="utf-8"))
        receipts = []
        for entry in entries:
            receipts.append(
                promote(
                    Path(entry["old"]),
                    Path(entry["new"]),
                    Path(args.db),
                    entry.get("new_home_id"),
                    args.now,
                )
            )
        print(json.dumps(receipts, indent=2))
        return 0
    except Refusal as exc:
        print(f"refused: {exc}", file=sys.stderr)
        return 2
    except (OSError, json.JSONDecodeError, KeyError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 3


if __name__ == "__main__":
    raise SystemExit(main())
