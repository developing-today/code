#!/usr/bin/env python3
"""Streaming repair filter for libsql /dump output (issue #1391).

Reads a SQL dump on stdin and writes it to stdout, passing every line
through byte-identical except one statement form: an
``INSERT INTO sqlite_schema(...)`` whose final value is the schema
object's SQL text. libsql's dump registers virtual tables that way
(inside a ``PRAGMA writable_schema=ON; ... OFF;`` block), but emits
the embedded SQL verbatim — single quotes inside it (an FTS5
``content='chunks'`` is the common case) are NOT doubled, so the
string literal terminates early and stock SQLite parsers fail with
``near "chunks": syntax error``.

Detection is parse-based and conservative: the statement is rewritten
only when its final literal cannot be a valid SQL string literal as
emitted (it closes early with trailing junk, or never closes). The
rewrite takes the raw text between the literal's opening quote and
the statement's closing ``');`` and doubles every single quote in it,
which is provably the correct encoding of that raw text; the result
is re-parsed before being emitted, and any statement that does not
verify is passed through unchanged. Correctly-escaped statements, and
statements whose SQL carries no quotes, pass through untouched.

The filter is line-oriented and constant-memory: it buffers only the
lines of one candidate statement. A candidate ends at a line ending
in ``');`` only when that line really closes the statement (#6531):
either the buffered text already parses as a valid statement ending
exactly at that line, or (a mis-escaped literal, which cannot be
parsed) the next line begins a new top-level statement or the input
ends. A line ending in ``');`` inside the embedded SQL — a trigger
body's ``SELECT RAISE(ABORT, 'bad');`` — therefore does not split the
statement. An unterminated statement at EOF is flushed unchanged —
the caller's truncation guard judges the filtered stream.

One bound keeps the constant-memory promise honest (#6525): a
candidate that grows past MAX_CANDIDATE_BYTES without terminating
is flushed unchanged and the rest of its statement streams through
line by line (the end is still recognised, so the following
statement is processed normally). Such a statement is not repaired
— repair requires holding it — but an unterminated or pathological
candidate can no longer grow the filter's memory to the size of
the remaining dump while emitting nothing.
"""

import os
import re
import sys

SCHEMA_INSERT = b"INSERT INTO sqlite_schema("
STATEMENT_END = b"');"
QUOTE = 0x27  # '
COMMA = 0x2C  # ,
SPACE = b" \t\r\n"
# A line that begins a new top-level dump statement (column 0).
# The largest candidate statement held for repair, in bytes. Real
# sqlite_schema statements (a table, view, trigger or index
# definition) are orders of magnitude smaller; the cap exists for
# the pathological candidate, not for them. Overridable for tests
# and for an operator repairing a genuinely huge schema.
def _max_candidate_bytes():
    raw = os.environ.get("REPAIR_DUMP_MAX_CANDIDATE_BYTES")
    if raw:
        try:
            value = int(raw)
        except ValueError:
            value = 0
        if value > 0:
            return value
    return 8 * 1024 * 1024


MAX_CANDIDATE_BYTES = _max_candidate_bytes()

TOP_LEVEL_START = re.compile(
    rb"(?:INSERT|PRAGMA|COMMIT|BEGIN|CREATE|DELETE|UPDATE|DROP|ALTER|"
    rb"REPLACE|ANALYZE|VACUUM|--)",
    re.IGNORECASE,
)


def parse_literal(text, pos):
    """Parse a SQLite string literal opening at text[pos].

    Returns the index just past the closing quote, or None when the
    literal never closes. A doubled '' inside is an escaped quote.
    """
    i = pos + 1
    while i < len(text):
        if text[i] == QUOTE:
            if text[i + 1 : i + 2] == b"'":
                i += 2
                continue
            return i + 1
        i += 1
    return None


def skip_value(text, pos):
    """Skip one VALUES() element starting at pos.

    Returns the index of the top-level comma or closing paren that
    follows the value, or None when the value does not parse.
    """
    n = len(text)
    while pos < n and text[pos] in SPACE:
        pos += 1
    if pos >= n:
        return None
    if text[pos] == QUOTE:
        end = parse_literal(text, pos)
        if end is None:
            return None
        pos = end
    else:
        depth = 0
        while pos < n:
            c = text[pos]
            if c == 0x28:  # (
                depth += 1
            elif c == 0x29:  # )
                if depth == 0:
                    break
                depth -= 1
            elif c == COMMA and depth == 0:
                break
            pos += 1
    while pos < n and text[pos] in SPACE:
        pos += 1
    return pos


def repair_statement(core):
    """Repair one INSERT INTO sqlite_schema statement (no newline).

    Returns the statement unchanged unless its final SQL literal is
    verifiably mis-escaped; then returns it with the literal's quotes
    doubled.
    """
    stripped = core.lstrip()
    if not stripped.startswith(SCHEMA_INSERT):
        return core
    lead = core[: len(core) - len(stripped)]
    text = stripped
    vpos = text.find(b"VALUES(")
    if vpos < 0:
        return core
    pos = vpos + len(b"VALUES(")
    # The final value is the schema SQL; skip the four values before
    # it (type, name, tbl_name, rootpage), each followed by a comma.
    for _ in range(4):
        pos = skip_value(text, pos)
        if pos is None or pos >= len(text) or text[pos] != COMMA:
            return core
        pos += 1
    while pos < len(text) and text[pos] in SPACE:
        pos += 1
    if pos >= len(text) or text[pos] != QUOTE:
        return core
    end = parse_literal(text, pos)
    if end is not None and text[end:] == b");":
        return core  # a valid literal exactly as emitted: untouched
    if not text.endswith(STATEMENT_END):
        return core
    sql = text[pos + 1 : -len(STATEMENT_END)]
    fixed = text[: pos + 1] + sql.replace(b"'", b"''") + text[-len(STATEMENT_END) :]
    # Emit the rewrite only if it now parses as intended.
    end = parse_literal(fixed, pos)
    if end is None or fixed[end:] != b");":
        return core
    return lead + fixed


def repair_chunk(chunk):
    """Repair one buffered statement, preserving its trailing newline."""
    newline = b""
    core = chunk
    if core.endswith(b"\n"):
        core, newline = core[:-1], b"\n"
        if core.endswith(b"\r"):
            core, newline = core[:-1], b"\r\n"
    return repair_statement(core) + newline


def ends_statement(chunk, next_line):
    """Whether the buffered chunk (last line ends in ``');``) is complete.

    A valid literal closing exactly at the chunk end is definitive.
    Otherwise the chunk may be a mis-escaped statement (unparseable by
    design), so it ends only where a new top-level statement follows,
    or at EOF (next_line is None).
    """
    text = chunk.rstrip(b"\r\n").lstrip()
    if text.startswith(SCHEMA_INSERT):
        pos = _final_literal_start(text)
        if pos is not None:
            end = parse_literal(text, pos)
            if end is not None and text[end:] == b");":
                return True
    if next_line is None:
        return True
    return bool(
        next_line.lstrip().startswith(SCHEMA_INSERT)
        or TOP_LEVEL_START.match(next_line)
    )


def _final_literal_start(text):
    """Index of the opening quote of the final VALUES() literal, or None."""
    vpos = text.find(b"VALUES(")
    if vpos < 0:
        return None
    pos = vpos + len(b"VALUES(")
    for _ in range(4):
        pos = skip_value(text, pos)
        if pos is None or pos >= len(text) or text[pos] != COMMA:
            return None
        pos += 1
    while pos < len(text) and text[pos] in SPACE:
        pos += 1
    if pos >= len(text) or text[pos] != QUOTE:
        return None
    return pos


def main():
    out = sys.stdout.buffer
    buffered = None
    buffered_bytes = 0
    passthrough = False
    lines = iter(sys.stdin.buffer)
    line = next(lines, None)
    while line is not None:
        next_line = next(lines, None)
        if passthrough:
            # The rest of an over-cap candidate streams through
            # unchanged; only its end is still recognised (the same
            # boundary test, applied to the current line), so the
            # statement after it is processed normally.
            out.write(line)
            out.flush()
            if line.rstrip(b"\r\n").endswith(STATEMENT_END) and ends_statement(
                line, next_line
            ):
                passthrough = False
        elif buffered is not None:
            buffered.append(line)
            buffered_bytes += len(line)
            if buffered_bytes > MAX_CANDIDATE_BYTES:
                # Over the cap: flush what was buffered unchanged
                # and stream the rest of the statement (#6525).
                for part in buffered:
                    out.write(part)
                out.flush()
                buffered = None
                buffered_bytes = 0
                passthrough = True
            elif line.rstrip(b"\r\n").endswith(STATEMENT_END):
                chunk = b"".join(buffered)
                if ends_statement(chunk, next_line):
                    out.write(repair_chunk(chunk))
                    buffered = None
                    buffered_bytes = 0
        elif line.lstrip().startswith(SCHEMA_INSERT):
            if line.rstrip(b"\r\n").endswith(STATEMENT_END) and ends_statement(
                line, next_line
            ):
                out.write(repair_chunk(line))
            elif len(line) > MAX_CANDIDATE_BYTES:
                out.write(line)
                passthrough = True
            else:
                buffered = [line]
                buffered_bytes = len(line)
        else:
            out.write(line)
        line = next_line
    if buffered is not None:
        out.write(b"".join(buffered))
    out.flush()


if __name__ == "__main__":
    main()
