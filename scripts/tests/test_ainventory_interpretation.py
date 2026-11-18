#!/usr/bin/env python3
"""Contract tests for scripts/ainventory-interpretation.py (#1248 slice 2,
slice tracker #10040).

Covers the two halves slice 2 lands:

1. the argument contract (repo #3352): unknown flags, stray positionals,
   present-but-valueless options and `--opt=value` handling are all
   decided by the parser before any network or file write;
2. the findings-store adapter: structured entries map to the event store,
   malformed entries are counted rather than imported as empty findings,
   and `--dry-run` writes nothing.

No test touches the network: every case either fails at parse time or
runs a `--dry-run` path.
"""

import importlib.util
import io
import json
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout, redirect_stderr
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "ainventory-interpretation.py"


def load_module():
    spec = importlib.util.spec_from_file_location("ainventory_interpretation", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


STORE = json.dumps(
    [
        {
            "heading": "GPT-6 Luna max — context band edge",
            "subject": "GPT-6 Luna max — context band edge",
            "finding": "Luna charges double above the 200k context band.",
            "evidence": "provider pricing page, 2026-10-27; price_events split at 200000 tokens.",
            "significance": "seat valuation assumes one band.",
            "action": "record the band edge in data, not code.",
            "severity": "watch",
            "model": "GPT-6 Luna",
        },
        {
            "heading": "Claude Haiku 5.5 — latency",
            "finding": "Haiku 5.5 TTFT regressed after the 2026-10-16 checkpoint.",
            "evidence": "three AA snapshots in a row.",
            "action": "re-measure next week.",
        },
        {"heading": "Entry with no finding field", "evidence": "unstructured"},
    ]
)


class ParserCases(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.module = load_module()

    def test_build_parser_rejects_unknown_flag(self):
        with redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as ctx:
                self.module.main(["awaiting", "--nope"])
        self.assertEqual(ctx.exception.code, 2)

    def test_build_parser_rejects_valueless_option(self):
        with redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as ctx:
                self.module.main(["record", "--batch", "--author", "x"])
        self.assertEqual(ctx.exception.code, 2)

    def test_equals_form_matches_space_form(self):
        parser = self.module.build_parser()
        a = parser.parse_args(["awaiting", "--limit=7"])
        b = parser.parse_args(["awaiting", "--limit", "7"])
        self.assertEqual(a.limit, b.limit)
        self.assertEqual(a.limit, 7)

    def test_stray_positional_is_usage_error(self):
        with redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as ctx:
                self.module.main(["awaiting", "extra"])
        self.assertEqual(ctx.exception.code, 2)

    def test_nonpositive_limit_rejected(self):
        with redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as ctx:
                self.module.main(["awaiting", "--limit", "0"])
        self.assertEqual(ctx.exception.code, 2)


class FindingsStoreCases(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.module = load_module()

    def test_parse_two_entries_one_malformed(self):
        entries, malformed = self.module.parse_findings_store(STORE)
        self.assertEqual(len(entries), 2)
        self.assertEqual(malformed, 1)
        first = entries[0]
        self.assertEqual(first["subject"], "GPT-6 Luna max — context band edge")
        self.assertIn("double above the 200k", first["finding"])
        self.assertEqual(first["severity"], "watch")
        self.assertEqual(first["model"], "GPT-6 Luna")
        # An unlabelled continuation line joins the field it follows.
        self.assertIn("split at 200000 tokens", first["evidence"])
        second = entries[1]
        self.assertTrue(second["finding"].startswith("Haiku 5.5 TTFT"))
        # A heading with no Subject field becomes the subject.
        self.assertEqual(second["subject"], "Claude Haiku 5.5 — latency")

    def test_entry_without_finding_is_not_imported(self):
        entries, _ = self.module.parse_findings_store(STORE)
        for entry in entries:
            self.assertTrue(entry.get("finding"))

    def test_dry_run_writes_nothing_and_imports_parsed_entries(self):
        with tempfile.TemporaryDirectory(prefix="findings-") as tmp:
            store = Path(tmp) / "findings-event-store.md"
            store.write_text(STORE)
            before = set(Path(tmp).iterdir())
            buf = io.StringIO()
            with redirect_stdout(buf):
                code = self.module.main(
                    ["import-findings", "--file", str(store), "--dry-run"]
                )
            self.assertEqual(code, 0)
            self.assertEqual(set(Path(tmp).iterdir()), before)
            report = json.loads(buf.getvalue())
            self.assertTrue(report["dry_run"])
            self.assertEqual(report["parsed"], 2)
            self.assertEqual(report["malformed"], 1)
            self.assertEqual(len(report["would_import"]), 2)
            refs = {row["source_ref"] for row in report["would_import"]}
            self.assertEqual(len(refs), 2)
            self.assertTrue(all(str(store) in ref for ref in refs))

    def test_empty_array_is_a_failure_not_a_silent_success(self):
        with tempfile.TemporaryDirectory(prefix="findings-") as tmp:
            store = Path(tmp) / "empty.json"
            store.write_text("[]\n")
            with self.assertRaises(SystemExit) as ctx:
                self.module.main(["import-findings", "--file", str(store), "--dry-run"])
            self.assertNotEqual(ctx.exception.code, 0)
            self.assertIn("no findings entries", str(ctx.exception.code))

    def test_markdown_source_is_refused_rather_than_guessed(self):
        with tempfile.TemporaryDirectory(prefix="findings-") as tmp:
            store = Path(tmp) / "findings-event-store.md"
            store.write_text("# Findings event store\n\n## Entry\n- finding: x\n")
            with self.assertRaises(SystemExit) as ctx:
                self.module.main(["import-findings", "--file", str(store), "--dry-run"])
            self.assertNotEqual(ctx.exception.code, 0)
            self.assertIn("canonical store", str(ctx.exception.code))

    def test_record_dry_run_needs_no_network(self):
        with tempfile.TemporaryDirectory(prefix="findings-") as tmp:
            note = Path(tmp) / "note.md"
            note.write_text("Batch note: 5 measurements, 1 price, no new models.")
            followups = Path(tmp) / "followups.json"
            followups.write_text(
                json.dumps(
                    [
                        {
                            "model": "Qwen4 Max",
                            "status": "not_filed",
                            "reason": "already covered by #1249",
                        }
                    ]
                )
            )
            buf = io.StringIO()
            with redirect_stdout(buf):
                code = self.module.main(
                    [
                        "record",
                        "--batch",
                        "00000000-0000-0000-0000-000000000000",
                        "--note-file",
                        str(note),
                        "--author",
                        "test",
                        "--followups",
                        str(followups),
                        "--dry-run",
                    ]
                )
            self.assertEqual(code, 0)
            plan = json.loads(buf.getvalue())
            self.assertTrue(plan["dry_run"])
            self.assertEqual(plan["would_record"]["note_kind"], "agent_note")
            self.assertEqual(len(plan["would_record"]["followups"]), 1)

    def test_unfiled_followup_without_reason_is_a_usage_error(self):
        with tempfile.TemporaryDirectory(prefix="findings-") as tmp:
            note = Path(tmp) / "note.md"
            note.write_text("Batch note.")
            followups = Path(tmp) / "followups.json"
            followups.write_text(
                json.dumps([{"model": "Qwen4 Max", "status": "not_filed"}])
            )
            with self.assertRaises(SystemExit) as ctx:
                self.module.main(
                    [
                        "record",
                        "--batch",
                        "00000000-0000-0000-0000-000000000000",
                        "--note-file",
                        str(note),
                        "--author",
                        "test",
                        "--followups",
                        str(followups),
                        "--dry-run",
                    ]
                )
            self.assertNotEqual(ctx.exception.code, 0)
            self.assertIn("without a reason", str(ctx.exception.code))


class ScriptSurface(unittest.TestCase):
    def test_script_is_executable_and_help_works(self):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--help"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("awaiting", result.stdout)
        self.assertIn("import-findings", result.stdout)

    def test_header_states_the_argument_contract(self):
        header = SCRIPT.read_text().split('"""')[1]
        self.assertIn("Argument contract", header)
        self.assertIn("#3352", header)


if __name__ == "__main__":
    unittest.main()

class TursoAdapterTests(unittest.TestCase):
    """Oracle: concrete SQLite selection and INSERT RETURNING semantics."""
    def test_awaiting_and_insert(self):
        import sqlite3
        module=load_module()
        class DB:
            def __init__(self):
                self.c=sqlite3.connect(':memory:');self.c.row_factory=sqlite3.Row
                self.c.executescript('CREATE TABLE ainventory_ingest_batches(id,source_kind,source_ref,ran_at,inserted_counts,note); CREATE TABLE ainventory_interpretation_runs(id INTEGER PRIMARY KEY,ingest_batch_id); CREATE TABLE ainventory_notes(id INTEGER PRIMARY KEY,kind,body,author,created_at);')
            def query(self,sql,args=()): return [dict(r) for r in self.c.execute(sql,args)]
            def batch(self,statements,transaction=False):
                with self.c: return [self.query(sql,args) for sql,args in statements]
        db=DB()
        for id,kind,counts in [('yes','gatherer','{"price_events":2}'),('empty','gatherer','{}'),('other','watcher','{"price_events":3}')]:
            db.query('INSERT INTO ainventory_ingest_batches VALUES(?,?,NULL,?, ?,NULL)',[id,kind,'2026-11-18',counts])
        rows=module.rest(db,'GET','/v_batches_awaiting_interpretation?limit=20')
        self.assertEqual([r['id'] for r in rows],['yes'])
        self.assertEqual(rows[0]['inserted_counts'],{'price_events':2})
        result=module.rest(db,'POST','/notes',[{'kind':'agent_note','body':'test','author':'test'}])
        self.assertEqual(result,[{'id':1}])
        with self.assertRaises(ValueError): module.rest(db,'POST','/unknown',[{}])
