#!/usr/bin/env python3
"""Pinning tests for issue #6154: the external-comms pusher must never
silently drop a malformed outbox line.

Each case runs the real `record-external-send push` as a subprocess
with an isolated EXTERNAL_COMMS_STATE_DIR and a loopback-refused
Supabase URL (127.0.0.1:9), so valid records stay pending with retry
bookkeeping and no database is needed.

Run: python3 -m unittest test_push_quarantine -v   (from this dir)
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "record-external-send")
TORN = '{"id":"torn-record","identity":"drew","channel":"emai'
VALID = {"id": "valid-record", "identity": "drew", "channel": "email",
         "destination": "x@example.com", "kind": "bug-report",
         "approval_basis": "per-item", "summary": "control record",
         "sent_at": "2026-11-05T00:00:00+00:00", "_attempts": 0}


class PushCase(unittest.TestCase):
    def run_push(self, lines):
        state = tempfile.mkdtemp(prefix="extcomms-test-")
        with open(os.path.join(state, "outbox.jsonl"), "w") as f:
            for ln in lines:
                f.write(ln + "\n")
        env = dict(os.environ,
                   EXTERNAL_COMMS_STATE_DIR=state,
                   EXTERNAL_COMMS_SUPABASE_URL="http://127.0.0.1:9",
                   EXTERNAL_COMMS_SERVICE_KEY="dummy-test-key")
        out = subprocess.run([sys.executable, SCRIPT, "push"], env=env,
                             capture_output=True, text=True, timeout=60)
        self.assertEqual(out.returncode, 0, out.stderr)
        health = {}
        hpath = os.path.join(state, "pusher-health.json")
        if os.path.exists(hpath):
            with open(hpath) as f:
                health = json.load(f)
        quarantine = []
        qpath = os.path.join(state, "quarantine.jsonl")
        if os.path.exists(qpath):
            with open(qpath) as f:
                quarantine = [json.loads(ln) for ln in f if ln.strip()]
        with open(os.path.join(state, "outbox.jsonl")) as f:
            outbox = [ln for ln in f if ln.strip()]
        return json.loads(out.stdout.strip()), health, quarantine, outbox

    def test_torn_only_is_quarantined_and_not_a_clean_drain(self):
        summary, health, quarantine, outbox = self.run_push([TORN])
        self.assertEqual(summary["quarantined"], 1)
        self.assertEqual(summary["pending"], 0)
        self.assertEqual(outbox, [])
        # the exact torn line is recoverable from the state dir
        self.assertEqual(len(quarantine), 1)
        self.assertEqual(quarantine[0]["_raw"], TORN)
        self.assertTrue(quarantine[0]["_reason"].startswith("json-decode:"))
        self.assertTrue(quarantine[0]["_quarantined_at"])
        # health accounts for the loss; no clean-drain claim
        self.assertEqual(health["last_run_quarantined"], 1)
        self.assertEqual(health["total_quarantined"], 1)
        self.assertEqual(health["last_quarantine_reason"],
                         quarantine[0]["_reason"])
        self.assertNotIn("last_drain_ok_at", health)
        self.assertEqual(health["last_lossy_drain_at"],
                         health["last_run_at"])

    def test_mixed_valid_and_torn(self):
        summary, health, quarantine, outbox = self.run_push(
            [json.dumps(VALID), TORN])
        self.assertEqual(summary["quarantined"], 1)
        self.assertEqual(summary["pending"], 1)
        # valid control stays pending with retry bookkeeping
        self.assertEqual(len(outbox), 1)
        kept = json.loads(outbox[0])
        self.assertEqual(kept["id"], "valid-record")
        self.assertEqual(kept["_attempts"], 1)
        self.assertIn("_last_error", kept)
        # torn line quarantined verbatim
        self.assertEqual([q["_raw"] for q in quarantine], [TORN])
        self.assertEqual(health["total_quarantined"], 1)

    def test_valid_only_control_has_no_quarantine(self):
        summary, health, quarantine, outbox = self.run_push(
            [json.dumps(VALID)])
        self.assertEqual(summary["quarantined"], 0)
        self.assertEqual(quarantine, [])
        self.assertEqual(health["last_run_quarantined"], 0)
        self.assertEqual(health["total_quarantined"], 0)
        self.assertEqual(len(outbox), 1)

    def test_non_object_json_line_is_quarantined(self):
        summary, health, quarantine, outbox = self.run_push(["[1, 2]"])
        self.assertEqual(summary["quarantined"], 1)
        self.assertEqual(quarantine[0]["_raw"], "[1, 2]")
        self.assertEqual(quarantine[0]["_reason"], "not a JSON object")
        self.assertEqual(outbox, [])


if __name__ == "__main__":
    unittest.main()
