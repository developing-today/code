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
lines of one candidate statement (which ends at a line ending in
``');``). An unterminated statement at EOF is flushed unchanged —
the caller's truncation guard judges the filtered stream.
"""

import sys

SCHEMA_INSERT = b"INSERT INTO sqlite_schema("
STATEMENT_END = b"');"
QUOTE = 0x27  # '
COMMA = 0x2C  # ,
SPACE = b" \t\r\n"


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


def main():
    out = sys.stdout.buffer
    buffered = None
    for line in sys.stdin.buffer:
        if buffered is not None:
            buffered.append(line)
            if line.rstrip(b"\r\n").endswith(STATEMENT_END):
                out.write(repair_chunk(b"".join(buffered)))
                buffered = None
            continue
        if line.lstrip().startswith(SCHEMA_INSERT):
            if line.rstrip(b"\r\n").endswith(STATEMENT_END):
                out.write(repair_chunk(line))
            else:
                buffered = [line]
            continue
        out.write(line)
    if buffered is not None:
        out.write(b"".join(buffered))
    out.flush()


if __name__ == "__main__":
    main()
