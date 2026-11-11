#!/usr/bin/env python3
"""Run a SQL file against the Turso DB `fleet-mirror` (repointed
2026-11-06 from muse-dev2 Supabase; PG version kept as run_sql_pg.py).
Usage: run_sql.py <sql-file>

The job's SQL files are written in the old PG dialect; this runner
translates exactly the constructs they use:
  agent.mirror_*  / agent.mirror_history_*   -> agent_mirror_* names
  now()                                      -> current UTC ISO timestamp
  DATE 'YYYY-MM-DD'                          -> 'YYYY-MM-DD'
Statements run one by one over the libsql pipeline (see
turso_client.py); SELECT results print as JSON lines so callers can
read reconciliation counts from stdout.
"""
import json
import re
import sys
from datetime import datetime, timezone

sys.path.insert(0, __file__.rsplit("/", 1)[0])
import turso_client


def translate(sql):
    now = datetime.now(timezone.utc).isoformat()
    sql = re.sub(r"\bagent\.(mirror(?:_history)?_[a-z0-9_]+)", r"agent_\1", sql)
    sql = re.sub(r"\bnow\(\)", f"'{now}'", sql)
    sql = re.sub(r"(?i)\bDATE\s+('[^']+')", r"\1", sql)
    return sql


def split_statements(sql):
    out, buf = [], ""
    for line in sql.splitlines():
        if line.strip().startswith("--"):
            continue
        buf += line + "\n"
        if line.rstrip().endswith(";"):
            stmt = buf.strip().rstrip(";").strip()
            if stmt:
                out.append(stmt)
            buf = ""
    tail = buf.strip()
    if tail:
        out.append(tail)
    return out


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    with open(sys.argv[1]) as f:
        sql = translate(f.read())
    stmts = split_statements(sql)
    # make sure any landing tables the SQL touches exist (history tables
    # and shells were not part of the migration copy)
    names = sorted(set(re.findall(r"agent_mirror_[a-z0-9_]+", sql)))
    client = turso_client.client()
    pre = turso_client.ensure_tables_sql(names)
    if pre:
        client.execute(pre)
    for stmt in stmts:
        results = client.execute([(stmt, [])])
        for res in results:
            for row in res.get("rows", []):
                print(json.dumps(row, default=str))
    print(f"run_sql: {len(stmts)} statement(s) ok")


if __name__ == "__main__":
    main()
