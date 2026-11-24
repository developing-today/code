#!/usr/bin/env python3
"""Build 1/5 (#14186): init sourcing contract + laptop job-spool ctx.

Oracle: a POST /jobs carrying FLEET_CTX_* env persists exactly that
subset (ids/hashes only) in the spool's meta.json, serves it on poll,
and survives a daemon restart; pre-contract spools without ctx restore
with ctx == {}; a malformed ctx skips the entry per the established
skip contract. Non-FLEET_CTX_* env never lands in ctx.

Run with: python3 -m unittest discover -s pkgs/laptop-agent/tests
"""

import contextlib
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

AGENT = Path(__file__).resolve().parents[1] / "src" / "laptop_agent.py"


def _import_agent():
    with tempfile.NamedTemporaryFile("w", suffix="-token", delete=False) as f:
        f.write("ctx-14186-test-token\n")
        token_path = f.name
    os.environ.setdefault("LAPTOP_AGENT_TOKEN_FILE", token_path)
    sys.path.insert(0, str(AGENT.parent))
    import laptop_agent as la

    return la


class CtxExtractTest(unittest.TestCase):
    def test_extract_ctx_filters_prefix_and_strings(self):
        la = _import_agent()
        env = {
            "FLEET_CTX_TASK_ID": "23046",
            "FLEET_CTX_ORIGIN": "opencode",
            "PATH": "/bin",
            "SECRET_TOKEN": "shh",
            "FLEET_CTX_NUM": 123,
        }
        ctx = la._extract_ctx(env)
        self.assertEqual(
            ctx, {"FLEET_CTX_TASK_ID": "23046", "FLEET_CTX_ORIGIN": "opencode"}
        )

    def test_extract_ctx_non_dict_is_empty(self):
        la = _import_agent()
        self.assertEqual(la._extract_ctx(None), {})
        self.assertEqual(la._extract_ctx("x"), {})


class CtxPersistTest(unittest.TestCase):
    def _load(self, la, state_dir):
        la.JOBS.clear()
        err = io.StringIO()
        with patch.object(la, "STATE_DIR", Path(state_dir)):
            with contextlib.redirect_stderr(err):
                la.load_persisted_jobs()
        return err.getvalue()

    def tearDown(self):
        la_mod = sys.modules.get("laptop_agent")
        if la_mod is not None:
            la_mod.JOBS.clear()

    def test_ctx_round_trip(self):
        la = _import_agent()
        job = {
            "id": "ctx-job",
            "proc": None,
            "cwd": "/",
            "cmd_repr": "echo hi",
            "started_at": 100.0,
            "ended_at": 150.0,
            "exit": 0,
            "state": "done",
            "interrupted": False,
            "ctx": {
                "FLEET_CTX_TASK_ID": "23046",
                "FLEET_CTX_ORIGIN": "opencode",
                "FLEET_CTX_HOST": "amd",
                "FLEET_CTX_ID": "task-23046",
            },
            "_files": (),
        }
        with tempfile.TemporaryDirectory() as state_dir:
            with patch.object(la, "STATE_DIR", Path(state_dir)):
                la._write_meta(job)
            raw = json.loads(
                (Path(state_dir) / "jobs" / "ctx-job" / "meta.json").read_text()
            )
            self.assertEqual(raw["ctx"], job["ctx"])
            stderr = self._load(la, state_dir)
            self.assertEqual(stderr, "")
            restored = la.JOBS["ctx-job"]
            self.assertEqual(restored["ctx"], job["ctx"])

    def test_pre_contract_spool_restores_empty_ctx(self):
        la = _import_agent()
        meta = {
            "id": "old-job",
            "cwd": "/",
            "cmd": "echo old",
            "started_at": 100.0,
            "ended_at": 150.0,
            "exit": 0,
            "state": "done",
            "interrupted": False,
        }
        with tempfile.TemporaryDirectory() as state_dir:
            spool = Path(state_dir) / "jobs" / "old-job"
            spool.mkdir(parents=True)
            (spool / "meta.json").write_text(json.dumps(meta))
            (spool / "stdout").write_bytes(b"")
            (spool / "stderr").write_bytes(b"")
            stderr = self._load(la, state_dir)
            self.assertEqual(stderr, "")
            self.assertEqual(la.JOBS["old-job"]["ctx"], {})

    def test_malformed_ctx_skips(self):
        la = _import_agent()
        for bad in ("x", 123, ["a"], {"FLEET_CTX_TASK_ID": 123}):
            meta = {
                "id": "bad-job",
                "cwd": "/",
                "cmd": "test",
                "started_at": 100.0,
                "ended_at": 150.0,
                "exit": 0,
                "state": "done",
                "interrupted": False,
                "ctx": bad,
            }
            with tempfile.TemporaryDirectory() as state_dir:
                spool = Path(state_dir) / "jobs" / "bad-job"
                spool.mkdir(parents=True)
                (spool / "meta.json").write_text(json.dumps(meta))
                (spool / "stdout").write_bytes(b"")
                (spool / "stderr").write_bytes(b"")
                stderr = self._load(la, state_dir)
                self.assertNotIn("bad-job", la.JOBS)
                self.assertIn("bad-job", stderr)


if __name__ == "__main__":
    unittest.main()
