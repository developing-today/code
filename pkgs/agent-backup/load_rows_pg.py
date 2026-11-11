#!/usr/bin/env python3
"""Generic loader: read a JSON spec from stdin, build upsert SQL, and run it
against muse-dev Supabase via ~/tools/supabase-exec.py (proxy env loaded the
recorder way when absent). Used by the runtime-db-export job for every unit.

Spec (JSON on stdin):
{
  "table": "agent.mirror_scheduler_jobs",       // landing table
  "conflict": ["job_id"],                        // PK columns
  "jsonb_cols": ["payload"],                     // columns cast to jsonb
  "hash_payload": true,                          // add payload_hash=md5(payload::text) + exported_at
  "history_table": "agent.mirror_history_scheduler_jobs" | null,
  "mode": "update" | "nothing",                  // ON CONFLICT behavior (default "update")
  "epoch_day": {"day_epoch": "day"},             // optional: rename col, emit (to_timestamp(v*86400) AT TIME ZONE 'UTC')::date
  "epoch_hour": {"hour_epoch": "hour"},          // optional: rename col, emit to_timestamp(v*3600)
  "rows": [ {col: value, ...}, ... ]             // muse.db result rows, uniform columns
}

With history_table set (mutable mirrors only): before the upsert, the OLD
mirror version of every row whose payload_hash differs is appended to the
history table with valid_from = the old row's exported_at, valid_to = now().
Unchanged rows are not touched (the upsert carries a hash-difference WHERE).

Exit 0 on success (prints "loaded N rows into <table>"), 1 on failure
(prints the error tail to stderr). A failed call changes nothing else.
"""
import hashlib
import json
import os
import subprocess
import sys
import tempfile

SUPABASE_EXEC = os.path.expanduser("~/tools/supabase-exec.py")
PROXY_ENV_FILE = os.path.expanduser("~/tools/secrets.d/mcpx-daemon.env")
PROXY_KEYS = ("HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy",
              "ALL_PROXY", "all_proxy", "NO_PROXY", "no_proxy")


def flush_env():
    env = dict(os.environ)
    if not (env.get("HTTPS_PROXY") or env.get("https_proxy")):
        try:
            with open(PROXY_ENV_FILE) as f:
                for line in f:
                    k, sep, v = line.strip().partition("=")
                    if sep and k in PROXY_KEYS and k not in env:
                        env[k] = v.strip().strip('"').strip("'")
        except OSError:
            pass
    return env


def lit(v):
    if v is None:
        return "NULL"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return repr(v)
    return "'" + str(v).replace("'", "''") + "'"


def run_sql(sql):
    fd, fname = tempfile.mkstemp(suffix=".sql", prefix="mirror-load-")
    try:
        with os.fdopen(fd, "w") as f:
            f.write(sql)
        p = subprocess.run([sys.executable, SUPABASE_EXEC, fname],
                           capture_output=True, text=True, timeout=300,
                           env=flush_env())
        if p.returncode != 0:
            tail = (p.stderr or "").strip().splitlines()
            head = (p.stdout or "").strip().splitlines()
            err = tail[-1] if tail else (head[-1] if head else "unknown failure")
            raise SystemExit(f"supabase-exec exit {p.returncode}: {err}"[:400])
    finally:
        try:
            os.unlink(fname)
        except OSError:
            pass


def main():
    spec = json.load(sys.stdin)
    rows = [{k: v for k, v in r.items() if not k.startswith("_")}
            for r in (spec.get("rows") or [])]
    table = spec["table"]
    if not rows:
        print(f"loaded 0 rows into {table} (empty page)")
        return
    conflict = spec.get("conflict", "update")
    jsonb_cols = set(spec.get("jsonb_cols") or [])
    hash_payload = bool(spec.get("hash_payload"))
    mode = spec.get("mode", "update")
    epoch_maps = {}
    for spec_key, mult in (("epoch_day", 86400), ("epoch_hour", 3600)):
        m = spec.get(spec_key)
        if m:
            for src, dst in m.items():
                epoch_maps[src] = (dst, mult)
    cols = []
    for c in rows[0].keys():
        cols.append(epoch_maps[c][0] if c in epoch_maps else c)

    def tuple_sql(r):
        parts = []
        for c in rows[0].keys():
            v = r[c]
            if c in epoch_maps:
                if epoch_maps[c][1] == 86400:
                    # pin to UTC: the Supabase session TZ otherwise shifts
                    # the date one day early (found in the 2026-10-29 build)
                    parts.append(f"(to_timestamp({int(v)} * 86400)"
                                 " AT TIME ZONE 'UTC')::date")
                else:
                    parts.append(f"to_timestamp({int(v)} * {epoch_maps[c][1]})")
            elif c in jsonb_cols:
                parts.append(lit(json.dumps(v)) + "::jsonb")
            else:
                parts.append(lit(v))
        return "(" + ", ".join(parts) + ")"

    values = ",\n".join(tuple_sql(r) for r in rows)
    col_list = ", ".join(cols)
    hist = spec.get("history_table")

    if hist:
        if not hash_payload:
            raise SystemExit("history_table requires hash_payload")
        join = " AND ".join(f"m.{k} = i.{k}" for k in conflict)
        hist_sql = f"""
WITH incoming AS (SELECT * FROM (VALUES
{values}
) AS v({col_list}))
INSERT INTO {hist} ({', '.join(conflict)}, payload, payload_hash, valid_from, valid_to)
SELECT {', '.join('m.' + k for k in conflict)}, m.payload, m.payload_hash, m.exported_at, now()
FROM {table} m JOIN incoming i ON {join}
WHERE m.payload_hash <> md5(i.payload::text);
"""
        run_sql(hist_sql)

    extra_cols = ""
    select_cols = col_list
    if hash_payload:
        extra_cols = ", payload_hash, exported_at"
        select_cols = col_list + ", md5(payload::text), now()"
    elif spec.get("no_exported_at"):
        extra_cols = ""
        select_cols = col_list
    elif "exported_at" not in cols:
        extra_cols = ", exported_at"
        select_cols = col_list + ", now()"

    if hist:
        upsert_sql = f"""
WITH incoming AS (SELECT * FROM (VALUES
{values}
) AS v({col_list}))
INSERT INTO {table} ({col_list}{extra_cols})
SELECT {select_cols} FROM incoming
ON CONFLICT ({', '.join(conflict)}) DO UPDATE SET
  {', '.join(f"{c} = EXCLUDED.{c}" for c in cols if c not in conflict)},
  payload_hash = EXCLUDED.payload_hash, exported_at = now()
WHERE {table}.payload_hash <> EXCLUDED.payload_hash;
"""
    elif mode == "nothing":
        upsert_sql = f"""
INSERT INTO {table} ({col_list}{extra_cols})
VALUES
{values}
ON CONFLICT ({', '.join(conflict)}) DO NOTHING;
"""
        if extra_cols:
            # rebuild values with the extra columns appended per-tuple
            tup = []
            for r in rows:
                base = tuple_sql(r)[:-1]  # strip ')'
                if hash_payload:
                    pj = json.dumps(r.get("payload"))
                    h = hashlib.md5(pj.encode()).hexdigest()
                    tup.append(base + f", '{h}', now())")
                else:
                    tup.append(base + ", now())")
            upsert_sql = f"""
INSERT INTO {table} ({col_list}{extra_cols})
VALUES
{', '.join(tup)}
ON CONFLICT ({', '.join(conflict)}) DO NOTHING;
"""
    else:
        set_cols = [c for c in cols if c not in conflict]
        assignments = ", ".join(f"{c} = EXCLUDED.{c}" for c in set_cols)
        if hash_payload:
            assignments += ", payload_hash = EXCLUDED.payload_hash"
        if not spec.get("no_exported_at"):
            assignments += ", exported_at = now()"
        tup = []
        for r in rows:
            base = tuple_sql(r)[:-1]
            if hash_payload:
                pj = json.dumps(r.get("payload"))
                h = hashlib.md5(pj.encode()).hexdigest()
                tup.append(base + f", '{h}', now())")
            elif spec.get("no_exported_at"):
                tup.append(base + ")")
            else:
                tup.append(base + ", now())")
        upsert_sql = f"""
INSERT INTO {table} ({col_list}{extra_cols})
VALUES
{', '.join(tup)}
ON CONFLICT ({', '.join(conflict)}) DO UPDATE SET {assignments};
"""
    run_sql(upsert_sql)
    print(f"loaded {len(rows)} rows into {table}")


if __name__ == "__main__":
    main()
