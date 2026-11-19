#!/usr/bin/env python3
"""Drop a table block that a libsql /dump repeats byte for byte (#12000).

Reads a SQL dump on stdin and writes it to stdout. libsql's /dump can
emit a table twice: the atlas database's vector table
(``vec_snowflake_m_v2_768_f32``, which carries a ``libsql_vector_idx``
index) came out as ``CREATE TABLE IF NOT EXISTS ...`` plus all of its
rows, then the same ``CREATE TABLE IF NOT EXISTS ...`` and every row
again, in the same order. The live table has no duplicate keys, so a
plain sqlite3 rebuild stops on the second copy's first primary key.

The filter recognises a block as a ``CREATE TABLE [IF NOT EXISTS] <name>``
statement on one line followed by the ``INSERT INTO <name> VALUES``
lines right after it. For every table it keeps a running SHA-256 of
that first block's INSERT lines and their count. When a second
``CREATE TABLE`` for a table already seen arrives:

* the CREATE statement must be byte-identical to the first one, and
* the INSERT lines that follow are consumed without being written,
  and their hash and count must equal the first block's.

When both hold, the repeat is dropped and one line on stderr says so
(table name and row count only, never row values). When either does
not hold, the filter exits 1 with a message: rows that differ are a
real conflict, and dropping them silently would lose data. Every other
line passes through byte-identical.

Constant memory: one digest, one count and one CREATE line per table,
never the rows themselves.
"""

import hashlib
import re
import sys

CREATE = re.compile(
    rb'^CREATE TABLE (?:IF NOT EXISTS )?("(?:[^"]|"")+"|\'(?:[^\']|\'\')+\'|[A-Za-z_][A-Za-z0-9_]*)'
)


def table_name(token):
    """The bare table name of a CREATE TABLE name token."""
    if token[:1] in (b'"', b"'"):
        quote = token[:1]
        return token[1:-1].replace(quote + quote, quote)
    return token


def insert_prefixes(name):
    """The INSERT line prefixes a dump uses for this table."""
    quoted = b'"' + name.replace(b'"', b'""') + b'"'
    prefixes = [b"INSERT INTO " + quoted + b" VALUES"]
    if re.fullmatch(rb"[A-Za-z_][A-Za-z0-9_]*", name):
        prefixes.append(b"INSERT INTO " + name + b" VALUES")
    return tuple(prefixes)


class Block:
    """One table's CREATE statement and the INSERT lines right after it."""

    def __init__(self, name):
        self.name = name
        self.prefixes = insert_prefixes(name)
        self.create = []  # the CREATE statement's lines (it may span several)
        self.digest = hashlib.sha256()
        self.count = 0

    @property
    def label(self):
        return self.name.decode(errors="replace")

    def owns(self, line):
        return line.startswith(self.prefixes)

    def add(self, line):
        self.digest.update(line)
        self.count += 1

    def same_rows(self, other):
        return (self.count, self.digest.digest()) == (other.count, other.digest.digest())


def ends_statement(line):
    return line.rstrip(b"\r\n \t").endswith(b";")


def fail(message):
    sys.stderr.write(f"dedupe_dump: {message}\n")
    sys.exit(1)


def finish_repeat(original, again):
    if not original.same_rows(again):
        fail(
            f"table {original.label} is dumped twice with different rows "
            f"({original.count} rows, then {again.count}); not dropping either copy"
        )
    sys.stderr.write(
        f"dedupe_dump: dropped a repeated copy of table {original.label} "
        f"({again.count} rows identical to the first copy)\n"
    )


def main():
    out = sys.stdout.buffer
    first = {}  # table name -> its first Block
    block = None  # the Block whose CREATE or INSERT lines are being read
    original = None  # set while block is a repeat of this first Block
    in_create = False
    for line in sys.stdin.buffer:
        if block is not None:
            if in_create:
                block.create.append(line)
                in_create = not ends_statement(line)
                if not in_create and original is not None and block.create != original.create:
                    fail(
                        f"table {block.label} is created twice with different "
                        "definitions; not dropping either copy"
                    )
                if original is None:
                    out.write(line)
                continue
            if block.owns(line):
                block.add(line)
                if original is None:
                    out.write(line)
                continue
            if original is not None:
                finish_repeat(original, block)
            block = original = None
        match = CREATE.match(line)
        if match:
            name = table_name(match.group(1))
            block = Block(name)
            block.create.append(line)
            in_create = not ends_statement(line)
            if name in first:
                original = first[name]
                if not in_create and block.create != original.create:
                    fail(
                        f"table {block.label} is created twice with different "
                        "definitions; not dropping either copy"
                    )
                continue  # the repeated CREATE is dropped with its rows
            first[name] = block
        out.write(line)
    if block is not None and original is not None:
        if in_create:
            fail(f"table {block.label}: the dump ends inside a repeated CREATE statement")
        finish_repeat(original, block)
    out.flush()


if __name__ == "__main__":
    main()
