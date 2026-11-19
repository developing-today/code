#!/usr/bin/env python3
"""Describe the dump statement a failed sqlite3 rebuild stopped on (#12000).

Usage: dump_line_report.py DUMP_SQL LINE

When ``sqlite3 -bail`` stops with ``Error near line N: UNIQUE
constraint failed``, the failing dump is deleted with the rest of the
work dir, so the cause cannot be read afterwards. This reads the dump
once (streaming, constant memory) and prints one line that says which
table line N inserts into, how many dump lines repeat that exact
statement, and how many INSERTs into the same table share its first
value. Repeated identical statements point at a dump that returned
the same row twice; different statements sharing a first value point
at real conflicting rows. Row values are never printed: only the
table name, line numbers and counts.
"""

import re
import sys

INSERT = re.compile(rb'^INSERT INTO ("(?:[^"]|"")+"|\S+?) VALUES ?\(')


def first_value_prefix(line):
    """The statement text up to and including its first value, or None."""
    match = INSERT.match(line)
    if not match:
        return None, None
    rest = line[match.end() :]
    if rest[:1] == b"'":
        # A quoted first value: find its closing quote ('' is an escape).
        i = 1
        while i < len(rest):
            if rest[i] == 0x27:
                if rest[i + 1 : i + 2] == b"'":
                    i += 2
                    continue
                break
            i += 1
        end = i + 1
    else:
        comma = rest.find(b",")
        paren = rest.find(b")")
        cands = [c for c in (comma, paren) if c >= 0]
        end = min(cands) if cands else len(rest)
    # Keep the delimiter after the value, so 7 does not match 70.
    return match.group(1), line[: match.end() + end + 1]


def main(argv):
    if len(argv) != 3:
        print("usage: dump_line_report.py DUMP_SQL LINE", file=sys.stderr)
        return 2
    path, target = argv[1], int(argv[2])
    with open(path, "rb") as fh:
        wanted = None
        for number, line in enumerate(fh, 1):
            if number == target:
                wanted = line.rstrip(b"\n")
                break
    if wanted is None:
        print(f"line {target}: past the end of the dump")
        return 0
    table, prefix = first_value_prefix(wanted)
    if table is None:
        print(f"line {target}: not an INSERT statement ({len(wanted)} bytes)")
        return 0
    same, shared, first_same = 0, 0, None
    with open(path, "rb") as fh:
        for number, line in enumerate(fh, 1):
            line = line.rstrip(b"\n")
            if not line.startswith(prefix):
                continue
            shared += 1
            if line == wanted:
                same += 1
                if first_same is None:
                    first_same = number
    name = table.decode("utf-8", "replace")
    print(
        f"line {target}: INSERT into {name}; {same} identical statement(s)"
        f" (first at line {first_same}); {shared} INSERT(s) share its first value"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
