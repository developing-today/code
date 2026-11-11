#!/usr/bin/env python3
"""Generate the atlas-core parity fixtures (developing-today/code#5352).

Builds the fixture corpus (fixture_corpus.sql + fixture_spec.json) into
a throwaway SQLite DB, runs the PRODUCTION Python atlas_search.py
against it (LocalBackend, embed_query monkeypatched to the fixture
query vector), and records the exact output envelopes. It also records
the Turso branch SQL (branch_queries) for representative filter sets
and the full call sequences of turso_allowed_ids / turso_index_branch
against a recording fake backend -- the server-side contract atlas-core
must reproduce byte-for-byte.

Run on the laptop (amd-6), never the sandbox:

  LD_LIBRARY_PATH=<x86-64 gcc lib>:<x86-64 zlib lib> \
    ~/atlas/v3-venv/bin/python gen_core_fixtures.py

Output: fixtures/core-expectations.json (committed; the .db is not).
"""

import argparse
import importlib.util
import json
import os
import sqlite3
import struct
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
PY_CLI = os.path.expanduser("~/atlas/turso-atlas/scripts/atlas_search.py")

spec = importlib.util.spec_from_file_location("atlas_search", PY_CLI)
A = importlib.util.module_from_spec(spec)
spec.loader.exec_module(A)

import numpy as np  # noqa: E402

with open(os.path.join(HERE, "fixture_spec.json")) as f:
    FSPEC = json.load(f)
QVEC_RAW = FSPEC["qvec_raw"]


def build_db(path):
    with open(os.path.join(HERE, "fixture_corpus.sql")) as f:
        sql = f.read()
    con = sqlite3.connect(path)
    con.executescript(sql)
    for cid, vec in FSPEC["vectors"].items():
        con.execute(
            "INSERT INTO vec_snowflake_m_v2_768_f32 (chunk_id, vec) VALUES (?, ?)",
            (int(cid), struct.pack(f"<{len(vec)}f", *vec)),
        )
    con.commit()
    con.close()


def fixture_embed_query(text, model=None, embedder=None, trunc=0):
    """embed_query stand-in: the fixture qvec, normalized exactly as
    the real embed_query normalizes (float32, divide by float norm)."""
    v = np.array(QVEC_RAW, dtype=np.float32)
    if trunc:
        v = v[:trunc]
    n = float(np.linalg.norm(v))
    return v / n


A.embed_query = fixture_embed_query


def mkargs(**over):
    base = dict(
        query="how do we back up the database",
        mode="hybrid",
        k=10,
        pool=100,
        kind=None,
        topic=None,
        project=None,
        client=None,
        since=None,
        until=None,
        max_sensitivity="personal",
        group_by_session=False,
        expand=0,
        with_message=False,
        model=A.MODEL,
        embedder="auto",
        trunc=0,
        vector_backend="scan",
    )
    base.update(over)
    return argparse.Namespace(**base)


def search_cases(be):
    cases = []

    def add(name, **over):
        out = A.run_search(be, mkargs(**over))
        cases.append({"name": name, "envelope": out})

    add("hybrid_default")
    add("hybrid_secret_ceiling", max_sensitivity="secret-bearing")
    add("hybrid_shareable_ceiling", max_sensitivity="shareable")
    add("fts_default", mode="fts")
    add("vector_default", mode="vector")
    add("hybrid_kind_code", kind="code")
    add("hybrid_topic_nixos", topic="nixos")
    add(
        "hybrid_client_codex_dates",
        client="codex",
        since="2026-10-01",
        until="2026-10-31",
    )
    add("hybrid_until_before_sB", client="codex", until="2026-10-19")
    add("hybrid_grouped", group_by_session=True, k=2)
    add("hybrid_expand_message", expand=1, with_message=True, k=3)
    add("hybrid_pool2_k2", pool=2, k=2)
    return cases


def get_chunk_cases(be):
    cases = []
    for name, cid, neighbors in (
        ("get_chunk_secret", 5, 0),
        ("get_chunk_neighbors", 2, 1),
    ):
        ns = argparse.Namespace(chunk_id=cid, neighbors=neighbors)
        cases.append({"name": name, "envelope": A.run_get_chunk(be, ns)})
    return cases


def turso_fixtures():
    stub = object.__new__(A.TursoBackend)
    stub.vec_table = A.VEC_TABLE
    out = {}

    def filters_for(**over):
        return A.Filters(mkargs(**over))

    bq = []
    for name, fover, me, qj, pool in (
        ("defaults", {}, '"back" OR "database"', "[0.5, 0.25]", 100),
        (
            "all_filters_secret",
            dict(
                kind="code",
                topic="rust",
                client="codex",
                since="2026-10-01",
                until="2026-10-31",
                max_sensitivity="secret-bearing",
            ),
            '"back" OR "database"',
            "[0.5, 0.25]",
            100,
        ),
        ("fts_only", {}, '"back" OR "database"', None, 100),
        ("vector_only", {}, None, "[0.5, 0.25]", 100),
    ):
        stmts = A.branch_queries(stub, filters_for(**fover), me, qj, pool)
        bq.append({"name": name, "stmts": [{"sql": s, "params": p} for s, p in stmts]})
    out["branch_queries"] = bq

    class FakeBe:
        vec_table = A.VEC_TABLE

        def __init__(self):
            self.calls = []

        def execute(self, sql, args=()):
            self.calls.append({"sql": sql, "params": list(args)})
            if "vector_top_k" in sql:
                script = {3: [5, 1, 6], 6: [5, 1, 6, 2, 8, 3]}
                return [{"id": i} for i in script[args[2]]]
            if "vector_distance_cos" in sql and "chunk_id IN" in sql:
                dists = {1: 0.01, 2: 0.3, 8: 0.07}
                return [{"cid": c, "dist": dists[c]} for c in args[1:]]
            if "c.sensitivity" in sql:
                return [{"cid": i} for i in (1, 2, 3, 4, 7, 8)]
            if "c2.id" in sql:
                return [{"cid": i} for i in (4, 7)]
            raise AssertionError(f"unexpected SQL: {sql}")

    fb = FakeBe()
    allowed = A.turso_allowed_ids(fb, filters_for())
    out["allowed_ids_plain"] = {"calls": fb.calls, "result": sorted(allowed)}

    fb = FakeBe()
    allowed = A.turso_allowed_ids(fb, filters_for(kind="code"))
    out["allowed_ids_kind"] = {"calls": fb.calls, "result": sorted(allowed)}

    fb = FakeBe()
    ranked = A.turso_index_branch(fb, filters_for(), "[0.5, 0.25]", 3, "index-fill")
    out["index_fill"] = {"calls": fb.calls, "result": [[c, d] for c, d in ranked]}

    fb = FakeBe()
    # index-fixed at pool 3 would call depth 3; reuse the scripted depth-3
    # response: survivors = [1] after filtering.
    ranked = A.turso_index_branch(fb, filters_for(), "[0.5, 0.25]", 3, "index-fixed")
    out["index_fixed"] = {"calls": fb.calls, "result": [[c, d] for c, d in ranked]}
    return out


def main():
    tmp = tempfile.mkdtemp(prefix="atlas-core-fixture-")
    db = os.path.join(tmp, "fixture.db")
    build_db(db)
    be = A.LocalBackend(db)
    doc = {
        "comment": (
            "Recorded from the production Python atlas_search.py "
            "by gen_core_fixtures.py. atlas-core's parity tests "
            "must reproduce every envelope and SQL string here."
        ),
        "qvec_raw": QVEC_RAW,
        "search_cases": search_cases(be),
        "get_chunk_cases": get_chunk_cases(be),
        "turso": turso_fixtures(),
    }
    out_path = os.path.join(HERE, "core-expectations.json")
    with open(out_path, "w") as f:
        json.dump(doc, f, indent=1, ensure_ascii=False)
    print(f"wrote {out_path}")


if __name__ == "__main__":
    main()
