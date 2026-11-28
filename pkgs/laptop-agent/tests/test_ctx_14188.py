#!/usr/bin/env python3
"""Build 2/5 (#14188): the daemon's /exec trip log carries the context id.

Oracle: log_trip writes ctx_id when given a non-empty string and omits
the key otherwise (pre-contract callers' records are unchanged), and the
/exec handler derives it from the command's effective env (daemon env
merged with the request's spec env, request wins) via _extract_ctx.

Run with: python3 -m unittest discover -s pkgs/laptop-agent/tests
"""

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
        f.write("ctx-14188-test-token\n")
        token_path = f.name
    os.environ.setdefault("LAPTOP_AGENT_TOKEN_FILE", token_path)
    sys.path.insert(0, str(AGENT.parent))
    import laptop_agent as la

    return la


class TripCtxIdTest(unittest.TestCase):
    def _records(self, la, calls):
        with tempfile.TemporaryDirectory() as d:
            trips = Path(d) / "trips.jsonl"
            with patch.object(la, "TRIPS_FILE", trips):
                for kwargs in calls:
                    la.log_trip("trip1", "exec", "echo hi", 5, 0, 3, 0, **kwargs)
            return [json.loads(line) for line in trips.read_text().splitlines()]

    def test_ctx_id_recorded_when_present(self):
        la = _import_agent()
        (rec,) = self._records(la, [{"ctx_id": "task-22655"}])
        self.assertEqual(rec["ctx_id"], "task-22655")
        # Safety contract unchanged: no raw command text, no env.
        self.assertEqual(rec["cmd0"], "echo")
        self.assertNotIn("env", rec)

    def test_absent_or_empty_ctx_id_leaves_record_unchanged(self):
        la = _import_agent()
        recs = self._records(la, [{}, {"ctx_id": None}, {"ctx_id": ""}, {"ctx_id": 7}])
        for rec in recs:
            self.assertNotIn("ctx_id", rec)

    def test_effective_ctx_request_env_wins_over_daemon_env(self):
        la = _import_agent()
        merged = {"FLEET_CTX_ID": "daemon-own", "PATH": "/bin"}
        merged.update({"FLEET_CTX_ID": "task-22655"})
        self.assertEqual(la._extract_ctx(merged).get("FLEET_CTX_ID"), "task-22655")


if __name__ == "__main__":
    unittest.main()
