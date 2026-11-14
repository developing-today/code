#!/usr/bin/env python3
"""Property-based coverage for the repair_dump streaming filter (#6525).

Invariants (the filter contract in scripts/repair_dump.py):
- A correctly escaped sqlite_schema statement is byte-identical
  after the filter, at ANY size -- in particular when it grows past
  MAX_CANDIDATE_BYTES and streams through in pass-through mode
  (#6525): pass-through must not rewrite, split, or lose any byte.
- The statement boundary after an over-cap correctly escaped
  statement is recognised from the statement's own bytes, so the
  next (below-cap, mis-escaped) statement is still repaired --
  whatever the cap position, the line splits inside the statement,
  the blank lines after it, or the interior lines baiting boundary
  detection (a line ending in ``');``, or a line that parses
  standalone as a complete schema INSERT).

Oracle: a sequential model of the contract. The generator emits a
dump whose expected output is known by construction: the over-cap
statement and any blank lines pass unchanged, and the one broken
statement is replaced by its escaped form. Any disagreement
between model and main() is a counterexample. (One documented
heuristic is outside the model: a candidate is taken to end where
a new top-level statement follows -- so the generator never places
a top-level-looking line directly after an interior line ending in
``');``, the ambiguous shape that heuristic exists for.)

Stdlib-only (no hypothesis in this repo's Python tooling), like
pkgs/deploy/test_argv_property.py; a fixed seed keeps CI
deterministic, and a failing case prints its seed and case index.

Usage: python3 -m unittest tests.test_repair_dump_property
"""

import importlib.util
import io
import random
import sys
import unittest
from pathlib import Path

REPAIR = Path(__file__).resolve().parent.parent / "scripts" / "repair_dump.py"

BROKEN = (
    b"INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql)"
    b" VALUES('table','fts','fts',0,'CREATE VIRTUAL TABLE fts"
    b" USING fts5(text, content='chunks', content_rowid='id')');\n"
)
ESCAPED = BROKEN.replace(b"content='chunks'", b"content=''chunks''").replace(
    b"content_rowid='id'", b"content_rowid=''id''"
)

OPENER = (
    b"INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql)"
    b" VALUES('table','big','big',0,'CREATE TABLE big (\n"
)

FAKE_COMPLETE = (
    b"INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql)"
    b" VALUES(1,2,3,4," + b"'" * 4 + b");"
)
FAKE_CANDIDATE = (
    b"INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql) VALUES(1,2,3,4,''X'');"
)

# Interior fragments for the over-cap statement's final literal, in
# emitted (correctly escaped) form: every quote is doubled, so each
# fragment is safe inside the literal, while several are shaped to
# bait boundary detection.
FRAGMENTS = [
    b"x",
    b"col TEXT DEFAULT ''dflt'',",
    b"CHECK (x <> ''),",
    b"SELECT RAISE(ABORT, ''bad'');",
    b"tail'');",
    FAKE_COMPLETE,
    FAKE_CANDIDATE,
    b"-- a comment mentioning ''''); and ''quotes''",
    b"",
    b"y" * 137,
]

TOP_LEVEL_PREFIXES = (b"INSERT", b"PRAGMA", b"CREATE", b"--", b"BEGIN", b"COMMIT")


def load_module():
    spec = importlib.util.spec_from_file_location("repair_dump_under_test", REPAIR)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def run_filter(mod, data, cap):
    old_cap = mod.MAX_CANDIDATE_BYTES
    old_stdin, old_stdout = sys.stdin, sys.stdout
    out = io.BytesIO()
    mod.MAX_CANDIDATE_BYTES = cap
    sys.stdin = type("FakeStd", (), {"buffer": io.BytesIO(data)})()
    sys.stdout = type("FakeStd", (), {"buffer": out})()
    try:
        mod.main()
    finally:
        sys.stdin, sys.stdout = old_stdin, old_stdout
        mod.MAX_CANDIDATE_BYTES = old_cap
    return out.getvalue()


def generate_case(rng):
    lines = []
    for _ in range(rng.randint(6, 40)):
        frags = [rng.choice(FRAGMENTS) for _ in range(rng.randint(1, 3))]
        lines.append(b" ".join(frags) + b"\n")
    # Documented-heuristic guard (see module docstring): never place
    # a top-level-looking line directly after an interior line ending
    # in '); .
    guarded = []
    for line in lines:
        if (
            guarded
            and guarded[-1].rstrip(b"\r\n").endswith(b"');")
            and line.lstrip().startswith(TOP_LEVEL_PREFIXES)
        ):
            guarded.append(b"pad pad pad\n")
        guarded.append(line)
    statement = OPENER + b"".join(guarded) + b"');\n"
    cap = rng.choice(
        [64, 128, 256, 512, 1024, 2048, rng.randint(32, len(statement) - 1)]
    )
    blanks = b"\n" * rng.randint(0, 2)
    tail = b"PRAGMA writable_schema=OFF;\n" if rng.random() < 0.5 else b""
    data = statement + blanks + BROKEN + tail
    expected = statement + blanks + ESCAPED + tail
    return data, expected, cap


class TestRepairDumpProperty(unittest.TestCase):
    def test_over_cap_valid_statement_byte_identical_and_next_repaired(self):
        mod = load_module()
        rng = random.Random(6525)
        for case in range(300):
            data, expected, cap = generate_case(rng)
            with self.subTest(case=case, cap=cap):
                self.assertEqual(
                    run_filter(mod, data, cap),
                    expected,
                    f"seed 6525 case {case} cap {cap}",
                )


if __name__ == "__main__":
    unittest.main()
