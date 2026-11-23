"""Shared helpers for the fleet-queue backup tools.

The queue store is a libSQL/SQLite database (Turso in production). These
helpers treat it as what D1 sees: a SQLite database. Counts and the schema
fingerprint are the fidelity currency every queue-backup tool checks.
"""

import hashlib
import sqlite3


def open_readonly(path):
    """Open a database file without modifying it (no journal, no WAL)."""
    return sqlite3.connect(f"file:{path}?mode=ro", uri=True)


def user_tables(conn):
    rows = conn.execute(
        "SELECT name FROM sqlite_master WHERE type = 'table'"
        " AND name NOT LIKE 'sqlite_%' ORDER BY name"
    ).fetchall()
    return [row[0] for row in rows]


def table_counts(conn):
    return {
        name: conn.execute(f'SELECT COUNT(*) FROM "{name}"').fetchone()[0]
        for name in user_tables(conn)
    }


def schema_fingerprint(conn):
    """Canonical schema text: one line per table/index, whitespace-folded.

    Two databases with the same fingerprint carry the same schema for
    backup purposes, regardless of the file bytes that produced them.
    """
    rows = conn.execute(
        "SELECT type, name, tbl_name, sql FROM sqlite_master"
        " WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'"
        " ORDER BY type, name"
    ).fetchall()
    lines = [
        "|".join((kind, name, tbl, " ".join(sql.split())))
        for kind, name, tbl, sql in rows
    ]
    return "\n".join(lines)


def schema_sha256(conn):
    return hashlib.sha256(schema_fingerprint(conn).encode("utf-8")).hexdigest()


def dump_database(conn):
    """The canonical SQLite dump of the whole database (iterdump)."""
    return "\n".join(conn.iterdump()) + "\n"


def load_dump_text(text, path):
    """Load a SQL dump into a fresh database file and return the connection."""
    conn = sqlite3.connect(path)
    try:
        conn.executescript(text)
        conn.commit()
    except Exception:
        conn.close()
        raise
    return conn


def load_dump_file(dump_path, db_path):
    with open(dump_path, "r", encoding="utf-8") as handle:
        return load_dump_text(handle.read(), db_path)
