#!/usr/bin/env python3
"""Offline only: prepare_runtime.py STAGING_DIR OUTPUT_DIR. Refuses existing dir.
Builds reviewed-candidate DDL and SQLite staging. No destination/network writes.
"""

import argparse, json, re, sqlite3, datetime, csv
from pathlib import Path
from pg_copy_archive import quote

NOW = "(strftime('%Y-%m-%dT%H:%M:%fZ','now'))"
UUID = "(lower(hex(randomblob(4)))||'-'||lower(hex(randomblob(2)))||'-4'||substr(lower(hex(randomblob(2))),2)||'-'||substr('89ab',(random() & 3)+1,1)||substr(lower(hex(randomblob(2))),2)||'-'||lower(hex(randomblob(6))))"
NAMES = [
    "ainventory.ingest_batches",
    "ainventory.models",
    "ainventory.provider_policies",
    "ainventory.notes",
    "ainventory.findings",
    "ainventory.interpretation_runs",
    "ainventory.measurement_events",
    "ainventory.price_events",
    "alerts.incidents",
    "alerts.incident_events",
    "alerts.push_subscriptions",
    "alerts.notification_sends",
    "public.agg_actions",
    "public.agg_activity_gaps",
    "public.agg_activity_ingestion",
    "public.agg_store_documents",
    "public.agg_usage_snapshots",
    "public.orch_runs",
    "public.orch_items",
]


def target(t):
    return t.split(".", 1)[1] if t.startswith("public.") else t.replace(".", "_")


def expr(x):
    x = re.sub(r"::(?:text|jsonb|integer|double precision)", "", x)
    x = re.sub(r"= ANY \(ARRAY\[([^]]+)\]\)", r"IN (\1)", x)
    x = x.replace("btrim(", "trim(").replace("jsonb_typeof(", "json_type(")
    x = x.replace(
        "json_type(value) = 'number'", "json_type(value) IN ('integer','real')"
    )
    x = x.replace(
        "json_type((value -> 'low')) = 'number'",
        "json_type(value,'$.low') IN ('integer','real')",
    )
    x = x.replace(
        "json_type((value -> 'high')) = 'number'",
        "json_type(value,'$.high') IN ('integer','real')",
    )
    # PG regex digits and explicit numeric cast, exact nonnegative version predicate.
    x = x.replace(
        "((document ->> 'version')) ~ '^[0-9]+$'",
        "((document ->> 'version') <> '' AND (document ->> 'version') NOT GLOB '*[^0-9]*')",
    )
    x = x.replace(
        "(document ->> 'version') ~ '^[0-9]+$'",
        "((document ->> 'version') <> '' AND (document ->> 'version') NOT GLOB '*[^0-9]*')",
    )
    x = x.replace(
        "((document ->> 'version')) = document_version",
        "CAST((document ->> 'version') AS INTEGER) = document_version",
    )
    return x


def text_array(value):
    if not value.startswith("{") or not value.endswith("}"):
        raise ValueError("only 1D text arrays supported")
    if value == "{}":
        return []
    out = []
    i = 1
    end = len(value) - 1
    while i < end:
        quoted = value[i] == '"'
        buf = []
        if quoted:
            i += 1
        while i < end:
            c = value[i]
            i += 1
            if c == "\\":
                if i >= end:
                    raise ValueError("truncated array escape")
                buf.append(value[i])
                i += 1
            elif quoted and c == '"':
                break
            elif not quoted and c == ",":
                i -= 1
                break
            elif not quoted and c in "{}":
                raise ValueError("nested array unsupported")
            else:
                buf.append(c)
        else:
            if quoted:
                raise ValueError("unterminated quoted array")
        item = "".join(buf)
        out.append(item if quoted or item != "NULL" else None)
        if i < end:
            if value[i] != ",":
                raise ValueError("array delimiter expected")
            i += 1
            if i == end:
                raise ValueError("empty final array item")
    return out


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("staging", type=Path)
    p.add_argument("output", type=Path)
    a = p.parse_args()
    a.output.mkdir(mode=0o700)
    schemas = json.loads((a.staging / "schema-inventory.json").read_text())
    source = (a.staging / "schema-only.sql").read_text()
    src = sqlite3.connect(
        "file:" + str(a.staging / "recovery-complete.db") + "?mode=ro", uri=True
    )
    db = sqlite3.connect(a.output / "runtime.db")
    statements = []
    metadata = {}
    for name in NAMES:
        ddl = schemas[name]
        cols = []
        constraints = []
        types = {}
        extras = re.findall(
            r"ALTER TABLE ONLY " + re.escape(name) + r"\n    ADD CONSTRAINT ([^;]+);",
            source,
        )
        pk = next(
            (
                re.search(r"PRIMARY KEY \(([^)]+)\)", e)[1]
                for e in extras
                if "PRIMARY KEY" in e
            ),
            None,
        )
        for line in ddl.splitlines()[1:]:
            l = line.strip().rstrip(",")
            if l.startswith("CONSTRAINT "):
                constraints.append(expr(l))
                continue
            if not l or l.startswith(")"):
                continue
            col, rest = l.split(" ", 1)
            typ = re.split(r" DEFAULT | NOT NULL", rest)[0]
            types[col] = typ
            st = (
                "INTEGER"
                if typ in ("bigint", "integer", "boolean")
                else "REAL"
                if typ in ("numeric", "double precision")
                else "TEXT"
            )
            definition = quote(col) + " " + st
            if col == pk and typ in ("bigint", "integer"):
                definition += " PRIMARY KEY AUTOINCREMENT"
            elif col == pk:
                definition += " PRIMARY KEY"
            if "NOT NULL" in rest:
                definition += " NOT NULL"
            if "DEFAULT " in rest:
                d = rest.split("DEFAULT ", 1)[1].removesuffix(" NOT NULL")
                if d == "now()":
                    d = NOW
                elif d == "gen_random_uuid()":
                    d = UUID
                elif typ == "text[]":
                    d = "'[]'"
                else:
                    d = re.sub(r"::(?:text|jsonb)", "", d)
                definition += " DEFAULT " + d
            cols.append(definition)
        for e in extras:
            if "PRIMARY KEY" in e:
                continue
            e = re.sub(
                r"REFERENCES ([\w.]+)", lambda m: "REFERENCES " + target(m[1]), e
            )
            constraints.append("CONSTRAINT " + e)
        statements.append(
            "CREATE TABLE "
            + target(name)
            + " (\n"
            + ",\n".join(cols + constraints)
            + "\n)"
        )
        metadata[name] = {"target": target(name), "types": types}
    # indexes preserve source uniqueness/lookup predicates with schema names mapped.
    for match in re.finditer(
        r"CREATE (UNIQUE )?INDEX ([\w]+) ON ([\w.]+) USING btree ([^;]+);", source
    ):
        if match[3] in NAMES:
            statements.append(
                "CREATE "
                + (match[1] or "")
                + "INDEX "
                + match[2]
                + " ON "
                + target(match[3])
                + " "
                + expr(match[4])
            )
    for table in [
        "findings",
        "interpretation_runs",
        "measurement_events",
        "price_events",
        "notes",
        "provider_policies",
    ]:
        statements.append(
            "CREATE TRIGGER ainventory_"
            + table
            + "_no_replace BEFORE INSERT ON ainventory_"
            + table
            + " WHEN EXISTS (SELECT 1 FROM ainventory_"
            + table
            + " WHERE id=NEW.id) BEGIN SELECT RAISE(ABORT,'append-only duplicate id'); END"
        )
        for verb in ["UPDATE", "DELETE"]:
            statements.append(
                "CREATE TRIGGER ainventory_"
                + table
                + "_no_"
                + verb.lower()
                + " BEFORE "
                + verb
                + " ON ainventory_"
                + table
                + " BEGIN SELECT RAISE(ABORT,'append-only event table'); END"
            )
    (a.output / "runtime-schema.sql").write_text(";\n".join(statements) + ";\n")
    for st in statements:
        db.execute(st)
    for name, meta in metadata.items():
        cols = list(meta["types"])
        sql = (
            "INSERT INTO "
            + target(name)
            + " ("
            + ",".join(quote(c) for c in cols)
            + ") VALUES ("
            + ",".join("?" for _ in cols)
            + ")"
        )
        rows = []
        for row in src.execute(
            "SELECT " + ",".join(quote(c) for c in cols) + " FROM " + quote(name)
        ):
            out = []
            for col, v in zip(cols, row):
                typ = meta["types"][col]
                if v is not None:
                    if typ == "timestamp with time zone":
                        v = (
                            datetime.datetime.fromisoformat(v)
                            .astimezone(datetime.timezone.utc)
                            .isoformat(timespec="milliseconds")
                            .replace("+00:00", "Z")
                        )
                    elif typ == "boolean":
                        v = 1 if v == "t" else 0 if v == "f" else int(v)
                    elif typ in ("integer", "bigint"):
                        v = int(v)
                    elif typ in ("numeric", "double precision"):
                        v = float(v)
                    elif typ == "text[]":
                        v = json.dumps(text_array(v))
                out.append(v)
            rows.append(out)
        db.executemany(sql, rows)
        meta["rows"] = len(rows)
    db.commit()
    assert db.execute("pragma integrity_check").fetchone() == ("ok",)
    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
    (a.output / "runtime-plan.json").write_text(json.dumps(metadata, indent=2))
    print(
        json.dumps(
            {
                "tables": len(metadata),
                "rows": sum(x["rows"] for x in metadata.values()),
                "integrity": "ok",
                "foreign_keys_verified": True,
            }
        )
    )


if __name__ == "__main__":
    main()
