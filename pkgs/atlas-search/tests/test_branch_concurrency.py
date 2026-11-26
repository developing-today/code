#!/usr/bin/env python3
"""Branch-concurrency tests for atlas_search.run_search (#12900 thread 1).

Oracle: the serial shape. A hybrid query's branches are independent
and join only at RRF fusion, so running them concurrently must not
change any output — and with slowed branch entry points the
concurrent run must finish well under the serial sum (proof the
branches actually overlap, not merely that both ran).

Cases:
- hybrid envelope under concurrency == envelope under a forced-serial
  run_branches == the committed recording in
  fixtures/core-expectations.json (the same recording atlas-core's
  parity tests hold the Rust port to).
- fts-only and vector-only envelopes (the inline single-branch shape)
  == their committed recordings.
- overlap timing with 0.6 s sleeps injected at both branch entries.
- die() inside a branch (SystemExit) reaches the caller with its
  exit code, on both the local and the turso branch paths.

Fixture rig mirrors fixtures/gen_core_fixtures.py: the fixture
corpus DB built in a temp dir, embed_query monkeypatched to the
fixture query vector. Run on the laptop with a numpy-carrying
python (the v3 venv; LD_LIBRARY_PATH per gen_core_fixtures.py):

  LD_LIBRARY_PATH=<x86-64 gcc lib>:<x86-64 zlib lib> \
    ~/atlas/v3-venv/bin/python tests/test_branch_concurrency.py
"""

import argparse
import importlib.util
import json
import os
import sqlite3
import struct
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
PKG = os.path.dirname(HERE)
FIXTURES = os.path.join(PKG, "fixtures")

spec = importlib.util.spec_from_file_location(
    "atlas_search", os.path.join(PKG, "atlas_search.py")
)
A = importlib.util.module_from_spec(spec)
spec.loader.exec_module(A)

import numpy as np  # noqa: E402

with open(os.path.join(FIXTURES, "fixture_spec.json")) as f:
    FSPEC = json.load(f)
QVEC_RAW = FSPEC["qvec_raw"]

with open(os.path.join(FIXTURES, "core-expectations.json")) as f:
    EXPECTED = {c["name"]: c["envelope"] for c in json.load(f)["search_cases"]}


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


def build_db(path):
    with open(os.path.join(FIXTURES, "fixture_corpus.sql")) as f:
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


def serial_run_branches(named):
    return {name: fn() for name, fn in named.items()}


class BranchConcurrencyTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="atlas-branch-test-")
        cls.db = os.path.join(cls.tmp, "fixture.db")
        build_db(cls.db)

    def backend(self):
        return A.LocalBackend(self.db)

    def test_hybrid_envelope_identical_serial_concurrent_recorded(self):
        be = self.backend()
        concurrent = A.run_search(be, mkargs())
        real = A.run_branches
        A.run_branches = serial_run_branches
        try:
            serial = A.run_search(be, mkargs())
        finally:
            A.run_branches = real
        self.assertEqual(serial, EXPECTED["hybrid_default"])
        self.assertEqual(concurrent, EXPECTED["hybrid_default"])

    def test_single_branch_envelopes_match_recorded(self):
        be = self.backend()
        self.assertEqual(A.run_search(be, mkargs(mode="fts")), EXPECTED["fts_default"])
        self.assertEqual(
            A.run_search(be, mkargs(mode="vector")), EXPECTED["vector_default"]
        )

    def test_branches_actually_overlap(self):
        be = self.backend()
        A.run_search(be, mkargs())  # warm the matrix cache
        orig_execute, orig_vs = be.execute, be.vector_search

        def slow_execute(sql, args=()):
            if "chunks_fts MATCH" in sql:
                time.sleep(0.6)
            return orig_execute(sql, args)

        def slow_vs(qvec, allowed, pool):
            time.sleep(0.6)
            return orig_vs(qvec, allowed, pool)

        be.execute, be.vector_search = slow_execute, slow_vs
        try:
            real = A.run_branches
            A.run_branches = serial_run_branches
            try:
                t0 = time.monotonic()
                A.run_search(be, mkargs())
                serial_s = time.monotonic() - t0
            finally:
                A.run_branches = real
            t0 = time.monotonic()
            A.run_search(be, mkargs())
            concurrent_s = time.monotonic() - t0
        finally:
            be.execute, be.vector_search = orig_execute, orig_vs
        # The injected sleeps really engaged on the serial shape...
        self.assertGreaterEqual(serial_s, 1.15)
        # ...and the concurrent shape overlaps them instead of summing.
        self.assertLess(concurrent_s, 1.0)

    def test_local_branch_die_exits_with_code(self):
        be = self.backend()
        orig_execute = be.execute

        def dying_execute(sql, args=()):
            if "chunks_fts MATCH" in sql:
                raise SystemExit(2)
            return orig_execute(sql, args)

        be.execute = dying_execute
        try:
            with self.assertRaises(SystemExit) as cm:
                A.run_search(be, mkargs())
            self.assertEqual(cm.exception.code, 2)
        finally:
            be.execute = orig_execute

    def test_turso_branch_die_exits_with_code(self):
        be = object.__new__(A.TursoBackend)
        be.vec_table = A.VEC_TABLE
        be.token = "unused"
        be.token_source = "test"

        def dying_pipeline(stmts):
            raise SystemExit(3)

        be.pipeline = dying_pipeline
        with self.assertRaises(SystemExit) as cm:
            A.run_search(be, mkargs())
        self.assertEqual(cm.exception.code, 3)


if __name__ == "__main__":
    unittest.main(verbosity=2)
