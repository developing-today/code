#!/usr/bin/env python3
"""Inspect a local SQLite schema/database artifact; never execute supplied SQL.

Argument contract: --database PATH (required), --help only; argparse rejects
unknown/abbreviated flags, stray positionals and missing values before file I/O.
Exit 0: no catalog gaps/unknowns; 1: incomplete evidence; 2: usage/open error.
This catalog-only tool never clears the live cutover gate.
"""

import argparse
from contextlib import closing
import json
from pathlib import Path
import sqlite3


DOMAINS = ("ainventory", "automation", "engineering", "finance", "home", "research")
TABLES = (
    tuple(f"{d}_{t}" for d in DOMAINS for t in ("sheet_rows", "ingest_batches"))
    + tuple(f"{d}_events" for d in DOMAINS if d != "ainventory")
    + ("engineering_records", "ideas_ideas")
)
FROZEN = {"automation", "finance", "research"}


def quoted(identifier):
    return '"' + identifier.replace('"', '""') + '"'


def contract(table):
    """Bounded repo-derived expectations, not a guessed canonical writer."""
    domain, kind = table.split("_", 1)
    if domain == "home":
        return set(), set(), []
    if kind == "ingest_batches":
        return (
            {
                "id",
                "source_kind",
                "source_ref",
                "ran_at",
                "inserted_counts",
                "note",
                "created_at",
            },
            {"source_kind", "ran_at", "inserted_counts", "created_at"},
            [("id",)],
        )
    if kind == "sheet_rows":
        columns = {"id", "source_id", "import_batch", "sheet_name", "row_index", "data"}
        return (
            columns,
            columns - {"id"},
            [("source_id", "import_batch", "sheet_name", "row_index")],
        )
    if table == "engineering_events":
        return (
            {
                "id",
                "run_date",
                "topic",
                "headline",
                "summary",
                "source_url",
                "source_name",
                "event_date",
                "dedupe_key",
                "payload",
                "created_at",
            },
            {"run_date", "topic", "headline", "dedupe_key", "payload", "created_at"},
            [("dedupe_key",)],
        )
    if domain in FROZEN and kind == "events":
        columns = {
            "id",
            "source_id",
            "import_batch",
            "sheet_name",
            "row_index",
            "kind",
            "record_id",
            "title",
            "status",
            "event_date",
            "source_ref",
            "topics",
            "fields",
            "dedupe_key",
            "created_at",
        }
        return (
            columns,
            {
                "source_id",
                "import_batch",
                "sheet_name",
                "row_index",
                "kind",
                "topics",
                "fields",
                "dedupe_key",
                "created_at",
            },
            [("import_batch", "sheet_name", "row_index"), ("dedupe_key",)],
        )
    # Curated tables have differing source DDL; do not generalize workbook fields.
    return {"id", "dedupe_key"}, {"dedupe_key"}, [("dedupe_key",)]


def inspect_database(path):
    uri = Path(path).resolve().as_uri() + "?mode=ro"
    with closing(sqlite3.connect(uri, uri=True)) as db:
        db.row_factory = sqlite3.Row
        db.execute("PRAGMA query_only=ON")
        db.execute("BEGIN")  # One consistent catalog snapshot, still read-only.
        catalog = [
            dict(row)
            for row in db.execute(
                "SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name"
            )
        ]
        tables = {}
        for name in TABLES:
            entry = next(
                (r for r in catalog if r["type"] == "table" and r["name"] == name), None
            )
            row = {
                "exists": entry is not None,
                "sql": entry["sql"] if entry else None,
                "columns": [],
                "indexes": [],
                "foreign_keys": [],
                "guards": [],
                "gaps": [],
                "unknown": ["guard behavior not exercised", "canonical writer unknown"],
            }
            tables[name] = row
            if name.startswith("home_"):
                row["unknown"].append("home contract unknown")
            if entry is None:
                row["gaps"].append("missing table")
                continue
            row["columns"] = [
                dict(r) for r in db.execute(f"PRAGMA table_info({quoted(name)})")
            ]
            row["foreign_keys"] = [
                dict(r) for r in db.execute(f"PRAGMA foreign_key_list({quoted(name)})")
            ]
            row["guards"] = [
                r for r in catalog if r["type"] == "trigger" and r["tbl_name"] == name
            ]
            unique = set()
            pk = tuple(
                r["name"]
                for r in sorted(row["columns"], key=lambda r: r["pk"])
                if r["pk"]
            )
            if pk:
                unique.add(pk)
            for index in db.execute(f"PRAGMA index_list({quoted(name)})").fetchall():
                item = dict(index)
                item["columns"] = [
                    dict(r)
                    for r in db.execute(f"PRAGMA index_info({quoted(item['name'])})")
                ]
                item["sql"] = next(
                    (
                        r["sql"]
                        for r in catalog
                        if r["type"] == "index" and r["name"] == item["name"]
                    ),
                    None,
                )
                row["indexes"].append(item)
                if item["unique"] and not item["partial"]:
                    unique.add(tuple(r["name"] for r in item["columns"]))
            expected, not_null, keys = contract(name)
            columns = {r["name"]: r for r in row["columns"]}
            if not name.startswith("home_"):
                if pk != ("id",):
                    row["gaps"].append("missing single-column id primary key")
                elif not columns["id"]["notnull"]:
                    rowid_alias = columns["id"][
                        "type"
                    ].upper() == "INTEGER" and not any(
                        i["origin"] == "pk" for i in row["indexes"]
                    )
                    if not rowid_alias:
                        row["gaps"].append("nullable primary identity: id")
            row["gaps"].extend(
                f"missing column: {c}" for c in sorted(expected - columns.keys())
            )
            row["gaps"].extend(
                f"nullable required column: {c}"
                for c in sorted(not_null & columns.keys())
                if not columns[c]["notnull"]
            )
            row["gaps"].extend(
                "missing unconditional unique key: " + ", ".join(key)
                for key in keys
                if key not in unique
            )
            if "source_id" in expected:
                source = name.split("_", 1)[0] + "_sources"
                if not any(
                    f["from"] == "source_id"
                    and f["table"] == source
                    and f["to"] == "id"
                    for f in row["foreign_keys"]
                ):
                    row["gaps"].append(
                        f"source association unenforced: source_id -> {source}.id"
                    )
            if name.endswith("_sheet_rows"):
                row["unknown"].append(
                    "batch association requires canonical writer and parent-key evidence"
                )
            if not row["guards"]:
                row["unknown"].append(
                    "no installed trigger guards observed; required guard policy unknown"
                )
        return {
            "evidence_scope": "local SQLite artifact only; not live target verification",
            "artifact": str(Path(path).resolve()),
            "live_gate_cleared": False,
            "tables": tables,
        }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument(
        "--database",
        required=True,
        help="existing local SQLite artifact (not SQL text)",
    )
    args = parser.parse_args(argv)
    try:
        report = inspect_database(args.database)
    except (sqlite3.Error, OSError, ValueError) as error:
        parser.exit(2, f"inspection failed: {error}\n")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 1  # Catalog evidence alone cannot clear guard/writer/live unknowns.


if __name__ == "__main__":
    raise SystemExit(main())
