#!/usr/bin/env python3
"""Keep a libsql vector index as a recorded statement (#12000).

Reads a SQL dump on stdin and writes it to stdout. libsql's /dump ends
a database with a vector index on a statement such as::

    CREATE INDEX vec_idx ON vec (libsql_vector_idx(v));

``libsql_vector_idx`` exists only in libsql, so the stock sqlite3 CLI
that rebuilds the export stops there ("no such function"), after every
row has loaded, and the whole export fails. The index's data is not
lost by skipping the statement: libsql keeps it in ordinary tables
(``<index>_shadow`` and ``libsql_vector_meta_shadow``), and those are
dumped and rebuilt like any other table.

So each such ``CREATE INDEX`` line is replaced by a row in a small
table, ``cold_store_libsql_vector_indexes(name, sql)``, holding the
statement exactly as libsql dumped it. Restoring into libsql: drop the
index's shadow table and its ``libsql_vector_meta_shadow`` row, then run
the recorded statement, and libsql rebuilds the index from the vectors.

Only a single-line ``CREATE INDEX`` whose text calls
``libsql_vector_idx(`` is rewritten. One that does not end on its line
fails the filter (exit 1) rather than being guessed at. Every other
line passes through byte-identical, so a dump without a vector index
comes out unchanged.
"""

import re
import sys

TABLE = b"cold_store_libsql_vector_indexes"
VECTOR_INDEX = re.compile(
    rb'^CREATE INDEX (?:IF NOT EXISTS )?("(?:[^"]|"")+"|[A-Za-z_][A-Za-z0-9_]*) ON .*'
    rb"\blibsql_vector_idx\s*\(",
    re.IGNORECASE,
)


def literal(raw):
    return b"'" + raw.replace(b"'", b"''") + b"'"


def index_name(token):
    if token[:1] == b'"':
        return token[1:-1].replace(b'""', b'"')
    return token


def main():
    out = sys.stdout.buffer
    created = False
    for line in sys.stdin.buffer:
        match = VECTOR_INDEX.match(line)
        if not match:
            out.write(line)
            continue
        statement = line.rstrip(b"\r\n")
        if not statement.rstrip(b" \t").endswith(b");"):
            sys.stderr.write(
                "vector_index_dump: a libsql vector index statement does not end "
                "on its line; not rewriting it\n"
            )
            sys.exit(1)
        name = index_name(match.group(1))
        if not created:
            out.write(
                b"CREATE TABLE IF NOT EXISTS "
                + TABLE
                + b"(name TEXT PRIMARY KEY, sql TEXT NOT NULL);\n"
            )
            created = True
        out.write(
            b"INSERT INTO "
            + TABLE
            + b" VALUES("
            + literal(name)
            + b","
            + literal(statement)
            + b");\n"
        )
        sys.stderr.write(
            f"vector_index_dump: kept libsql vector index {name.decode(errors='replace')} "
            f"as a recorded statement in {TABLE.decode()}\n"
        )
    out.flush()


if __name__ == "__main__":
    main()
