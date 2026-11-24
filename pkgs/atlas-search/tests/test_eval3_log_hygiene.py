#!/usr/bin/env python3
"""Log-hygiene tests for the frozen-v3 evaluator (eval3.py, #13599).

Oracle: a synthetic canary query run through the FTS runner must score
exactly as its fixture says, while the canary text appears in neither
stdout nor stderr. Progress lines still carry id, category, rank and
position, so aggregate progress is preserved without free-form text.

The frozen corpus is not checked in, so the rig builds a tiny synthetic
corpus (chunks + FTS5 table) in a temp dir and points the module's
fingerprint constants at it — fingerprint enforcement itself is
untouched; only the constants are re-pointed for the fixture corpus.

Run remotely (never locally per fleet rules), e.g. via the fleet
command workflow against the branch:

  python3 pkgs/atlas-search/tests/test_eval3_log_hygiene.py -v
"""

import hashlib
import importlib.util
import io
import json
import sqlite3
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
EVAL3 = HERE.parent / "eval" / "eval3.py"


def load_eval3():
    spec = importlib.util.spec_from_file_location("eval3", EVAL3)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


# Distinctive canary tokens: appear in the query and (for the hit) in
# exactly one chunk, nowhere else in the rig.
CANARY_HIT = "CANARY zqxj wombat quixotic fjord"
CANARY_MISS = "CANARY zxqv nowhere absent unheard"


class Eval3LogHygieneTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.eval3 = load_eval3()
        self.corpus = self.root / "corpus.db"
        self._build_corpus()
        self.queries = self.root / "queries.json"
        self.queries.write_text(
            json.dumps(
                {
                    "queries": [
                        {
                            "id": "canary-1",
                            "category": "canary",
                            "query": CANARY_HIT,
                            "chunk_ids": [2],
                        },
                        {
                            "id": "canary-2",
                            "category": "canary",
                            "query": CANARY_MISS,
                            "chunk_ids": [3],
                        },
                    ]
                }
            )
        )
        self.out = self.root / "res.json"

    def tearDown(self):
        self.tmp.cleanup()

    def _build_corpus(self):
        con = sqlite3.connect(self.corpus)
        con.execute("CREATE TABLE chunks (id INTEGER PRIMARY KEY, content_hash TEXT)")
        con.execute("CREATE VIRTUAL TABLE chunks_fts USING fts5(content)")
        chunks = {
            1: "ordinary passage about river stones and quiet mornings",
            2: "zqxj wombat quixotic fjord canary passage with rare words",
            3: "another ordinary passage about market stalls and rain",
        }
        for cid, text in chunks.items():
            con.execute(
                "INSERT INTO chunks (id, content_hash) VALUES (?, ?)",
                (cid, hashlib.sha256(text.encode()).hexdigest()),
            )
            con.execute(
                "INSERT INTO chunks_fts (rowid, content) VALUES (?, ?)", (cid, text)
            )
        con.commit()
        # Point the frozen fingerprint at this synthetic corpus, using
        # the module's own fingerprint rule (ids with id % 997 == 0).
        n = con.execute("SELECT count(*) FROM chunks").fetchone()[0]
        h = hashlib.sha256()
        for (ch,) in con.execute(
            "SELECT content_hash FROM chunks WHERE id % 997 = 0 ORDER BY id"
        ):
            h.update(ch.encode())
        con.close()
        self.eval3.EXPECTED_CHUNKS = n
        self.eval3.EXPECTED_SHA = h.hexdigest()

    def _run(self):
        argv = [
            "eval3.py",
            "--mode",
            "fts",
            "--corpus",
            str(self.corpus),
            "--queries",
            str(self.queries),
            "--out",
            str(self.out),
        ]
        old_argv = sys.argv
        sys.argv = argv
        try:
            stdout, stderr = io.StringIO(), io.StringIO()
            with redirect_stdout(stdout), redirect_stderr(stderr):
                self.eval3.main()
            return stdout.getvalue(), stderr.getvalue()
        finally:
            sys.argv = old_argv

    def test_canary_absent_from_output_and_score_matches_fixture(self):
        stdout, stderr = self._run()
        for secret in (CANARY_HIT, CANARY_MISS, "zqxj", "zxqv"):
            self.assertNotIn(secret, stdout, f"canary leaked to stdout: {secret}")
            self.assertNotIn(secret, stderr, f"canary leaked to stderr: {secret}")
        # Aggregate progress is still emitted: id, category, rank, position.
        self.assertIn("canary-1", stdout)
        self.assertIn("canary-2", stdout)
        self.assertIn("[1/2]", stdout)
        self.assertIn("[2/2]", stdout)
        # Scores still match the fixture: hit ranks 1, miss ranks null.
        result = json.loads(self.out.read_text())
        self.assertEqual(
            result["per_question"],
            [
                {"id": "canary-1", "category": "canary", "rank": 1},
                {"id": "canary-2", "category": "canary", "rank": None},
            ],
        )
        summary = result["summary"]
        self.assertEqual(summary["n"], 2)
        self.assertEqual(summary["recall@5"], 1)
        self.assertEqual(summary["recall@10"], 1)
        self.assertEqual(summary["mrr"], 0.5)


if __name__ == "__main__":
    unittest.main(verbosity=2)
