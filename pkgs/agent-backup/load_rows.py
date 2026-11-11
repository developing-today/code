#!/usr/bin/env python3
"""Generic loader: read a JSON spec from stdin, build upsert SQL, and run
it against the Turso DB `fleet-mirror` (repointed 2026-11-06 from
muse-dev2 Supabase; the PG version is kept alongside as load_rows_pg.py
for reference/rollback, and run_sql_pg.py / schema.sql document the old
landing side). Used by the runtime-db-export job for every unit.

Spec (JSON on stdin) — unchanged from the PG loader:
{
  "table": "agent.mirror_scheduler_jobs",       // landing table (spec
                    // name; physically lands in agent_mirror_* in Turso)
  "conflict": ["job_id"],                        // PK columns
  "jsonb_cols": ["payload"],                     // columns stored as JSON text
  "hash_payload": true,                          // add payload_hash=md5 of the
                    // stored JSON text + exported_at
  "history_table": "agent.mirror_history_scheduler_jobs" | null,
  "mode": "update" | "nothing",                  // ON CONFLICT behavior (default "update")
  "epoch_day": {"day_epoch": "day"},             // rename col, emit UTC date text
  "epoch_hour": {"hour_epoch": "hour"},          // rename col, emit UTC timestamp text
  "no_exported_at": true,                        // export_state only
  "rows": [ {col: value, ...}, ... ]             // muse.db result rows, uniform columns
}

With history_table set (mutable mirrors only): before the upsert, the OLD
mirror version of every row whose payload_hash differs is appended to the
history table with valid_from = the old row's exported_at, valid_to = now.
Unchanged rows are not touched (the upsert carries a hash-difference WHERE).

Shapes match the rows the 2026-11-06 Turso migration copy already holds:
dates as YYYY-MM-DD text, timestamps as ISO-8601 UTC text, JSON columns as
JSON text, booleans as 1/0. payload_hash is md5 of the JSON text stored
(the PG loader's history path hashed PG's canonical jsonb text instead;
key order and separators coincide for muse.db-built payloads, but the
first post-repoint run may still record a one-time history version-step
for rows whose texts differ — hashes are self-consistent from then on).

Exit 0 on success (prints "loaded N rows into <table>"), 1 on failure.
A failed call changes nothing else. Transport details (token by name,
pipeline, staging mode): see turso_client.py.
"""
import hashlib
import json
import sys
from datetime import datetime, timedelta, timezone

sys.path.insert(0, __file__.rsplit("/", 1)[0])
import turso_client


def iso_now():
    return datetime.now(timezone.utc).isoformat()


def epoch_text(v, mult):
    if v is None or isinstance(v, str):
        return v
    base = datetime(1970, 1, 1, tzinfo=timezone.utc)
    if mult == 86400:
        return (base + timedelta(days=int(v))).strftime("%Y-%m-%d")
    return (base + timedelta(hours=int(v))).isoformat()


def main():
    spec = json.load(sys.stdin)
    rows = [{k: v for k, v in r.items() if not k.startswith("_")}
            for r in (spec.get("rows") or [])]
    table_spec = spec["table"]
    table = turso_client.physical_name(table_spec)
    if not rows:
        print(f"loaded 0 rows into {table_spec} (empty page)")
        return
    conflict = spec.get("conflict") or []
    jsonb_cols = set(spec.get("jsonb_cols") or [])
    hash_payload = bool(spec.get("hash_payload"))
    mode = spec.get("mode", "update")
    hist = spec.get("history_table")
    if hist and not hash_payload:
        raise SystemExit("history_table requires hash_payload")
    hist_phys = turso_client.physical_name(hist) if hist else None

    epoch_maps = {}
    for spec_key, mult in (("epoch_day", 86400), ("epoch_hour", 3600)):
        m = spec.get(spec_key)
        if m:
            for src, dst in m.items():
                epoch_maps[src] = (dst, mult)
    cols = [epoch_maps[c][0] if c in epoch_maps else c for c in rows[0].keys()]
    src_for = {dst: src for src, (dst, _m) in epoch_maps.items()}

    now = iso_now()
    prepared = []
    for r in rows:
        vals = {}
        for c in cols:
            if c in src_for:
                src = src_for[c]
                vals[c] = epoch_text(r.get(src), epoch_maps[src][1])
            else:
                v = r.get(c)
                if c in jsonb_cols and v is not None:
                    v = v if isinstance(v, str) else json.dumps(v)
                vals[c] = v
        prepared.append(vals)

    extra = []
    if hash_payload:
        extra = ["payload_hash", "exported_at"]
    elif spec.get("no_exported_at"):
        extra = []
    elif "exported_at" not in cols:
        extra = ["exported_at"]
    for vals in prepared:
        if hash_payload:
            p = vals.get("payload")
            text = p if isinstance(p, str) else json.dumps(p)
            vals["payload_hash"] = hashlib.md5(text.encode()).hexdigest()
        if extra and "exported_at" in extra:
            vals["exported_at"] = now
    all_cols = cols + extra

    stmts = []
    stmts += turso_client.ensure_tables_sql([table] + ([hist_phys] if hist_phys else []))
    collist = ", ".join(all_cols)
    ph = ", ".join("?" for _ in all_cols)
    if hist_phys:
        # stage the page in a temp table for the history archive step
        # (SQLite upserts cannot source from SELECT, so the upsert
        # itself below is VALUES-based; the archive is a plain
        # INSERT..SELECT and can read the staged page)
        stmts.append((f"CREATE TEMP TABLE rde_incoming ({collist})", []))
        stmts.append(("DELETE FROM rde_incoming", []))
        for vals in prepared:
            stmts.append((f"INSERT INTO rde_incoming ({collist}) VALUES ({ph})",
                          [vals.get(c) for c in all_cols]))
        on = " AND ".join(f"m.{k} = i.{k}" for k in conflict)
        stmts.append((
            f"INSERT INTO {hist_phys} ({', '.join(conflict)}, payload, payload_hash, valid_from, valid_to) "
            f"SELECT {', '.join('m.' + k for k in conflict)}, m.payload, m.payload_hash, m.exported_at, ? "
            f"FROM {table} m JOIN rde_incoming i ON {on} "
            f"WHERE m.payload_hash <> i.payload_hash", [now]))
    values_sql = ", ".join(f"({ph})" for _ in prepared)
    values_args = []
    for vals in prepared:
        values_args += [vals.get(c) for c in all_cols]
    insert = f"INSERT INTO {table} ({collist}) VALUES {values_sql}"
    if hist_phys:
        set_cols = [c for c in all_cols if c not in conflict]
        sets = ", ".join(f"{c} = excluded.{c}" for c in set_cols)
        insert += (f" ON CONFLICT ({', '.join(conflict)}) DO UPDATE SET {sets}"
                   f" WHERE {table}.payload_hash <> excluded.payload_hash")
    elif mode == "nothing":
        insert += " ON CONFLICT DO NOTHING"
    else:
        set_cols = [c for c in all_cols if c not in conflict]
        if set_cols:
            sets = ", ".join(f"{c} = excluded.{c}" for c in set_cols)
            insert += f" ON CONFLICT ({', '.join(conflict)}) DO UPDATE SET {sets}"
        else:
            insert += " ON CONFLICT DO NOTHING"
    stmts.append((insert, values_args))
    if hist_phys:
        stmts.append(("DROP TABLE IF EXISTS rde_incoming", []))

    turso_client.client().execute(stmts)
    print(f"loaded {len(prepared)} rows into {table_spec}")


if __name__ == "__main__":
    main()
