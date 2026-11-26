"""Oracle: the PostgreSQL migration contract, as test-migration.sh and
test-orchestration-migration.sh state it for PostgreSQL, must hold
identically for a store the Turso runner creates — same tables, same
accepting and rejecting rows — and the runner must be idempotent and
must fail loudly (issue #14256: a failed run reads as failed).

The translation itself is pinned by golden files in ``golden/``: they
were generated once from the translator and reviewed against the
PostgreSQL sources; a future migration construct that changes the
translation breaks the golden test until the new golden is reviewed
and regenerated (``UPDATE_GOLDEN=1`` in the environment regenerates).
"""

import importlib.util
import json
import os
import sqlite3
import sys
import tempfile
import threading
import unittest
import unittest.mock
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))


def load_module(name, filename):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


tm = load_module("turso_migrations", "turso_migrations.py")
cli = load_module("apply_store_migrations", "apply-store-migrations.py")

MIGRATION_IDS = [mid for mid, _ in tm.load_migrations()]
AUTHOR = '{"kind":"drew","id":"drew","label":"Drew"}'
HASH = "a" * 64


def applied_client(test_case):
    tmp = tempfile.TemporaryDirectory()
    test_case.addCleanup(tmp.cleanup)
    client = tm.LocalSqliteClient(str(Path(tmp.name) / "store.db"))
    verdict = tm.apply_migrations(client, tm.load_migrations())
    test_case.assertIsNone(verdict["failed"])
    test_case.assertEqual(verdict["applied"], MIGRATION_IDS)
    return client


class TranslationTests(unittest.TestCase):
    def test_golden_translations(self):
        for mid, sql in tm.load_migrations():
            golden = (HERE / "golden" / f"{mid}.sqlite.sql").read_text()
            rendered = "\n".join(s + ";" for s in tm.translate(sql)) + "\n"
            if os.environ.get("UPDATE_GOLDEN"):
                (HERE / "golden" / f"{mid}.sqlite.sql").write_text(rendered)
            self.assertEqual(rendered, golden, f"translation drift in {mid}")

    def test_unknown_regex_is_refused(self):
        with self.assertRaises(tm.TranslationError):
            tm.translate("CREATE TABLE x (a text CHECK (a ~ '^x+$'));")

    def test_unknown_cast_is_refused(self):
        with self.assertRaises(tm.TranslationError):
            tm.translate("CREATE TABLE x (a text DEFAULT 'n'::uuid);")

    def test_unknown_jsonb_function_is_refused(self):
        with self.assertRaises(tm.TranslationError):
            tm.translate("CREATE TABLE x (a jsonb CHECK (jsonb_typeof(a) = 'array'));")


class ApplyTests(unittest.TestCase):
    def test_all_tables_created(self):
        client = applied_client(self)
        rows = client.query("SELECT name FROM sqlite_master WHERE type = 'table'")
        tables = {row["name"] for row in rows}
        self.assertTrue(
            {
                "agg_actions",
                "shared_events",
                "orch_runs",
                "orch_items",
                "agg_activity_ingestion",
                "agg_activity_gaps",
                "agg_usage_snapshots",
                "agg_store_documents",
                "shared_comments",
                tm.LEDGER_TABLE,
            }.issubset(tables)
        )

    def test_second_run_is_a_reported_noop(self):
        client = applied_client(self)
        verdict = tm.apply_migrations(client, tm.load_migrations())
        self.assertEqual(verdict["applied"], [])
        self.assertEqual(verdict["already_applied"], MIGRATION_IDS)
        self.assertIsNone(verdict["failed"])

    def test_ledger_rows_carry_a_timestamp(self):
        client = applied_client(self)
        rows = client.query(f"SELECT migration_id, applied_at FROM {tm.LEDGER_TABLE}")
        self.assertEqual({row["migration_id"] for row in rows}, set(MIGRATION_IDS))
        for row in rows:
            self.assertIn("T", row["applied_at"])
            self.assertTrue(row["applied_at"].endswith("Z"))


class ConstraintTests(unittest.TestCase):
    """Row-level probes mirroring the PostgreSQL suites' assertions."""

    def setUp(self):
        self.client = applied_client(self)
        self.db = self.client.db

    def ok(self, sql, args=()):
        self.db.execute(sql, args)
        self.db.commit()

    def refused(self, sql, args=()):
        with self.assertRaises(sqlite3.Error):
            self.db.execute(sql, args)
        self.db.rollback()

    def action(self, **overrides):
        row = {
            "id": "da-1",
            "client_token": None,
            "kind": "triage",
            "verb": "acknowledge",
            "object_ref": "orch:item-1",
            "payload": "{}",
            "author": AUTHOR,
            "created_at": "2026-11-25T00:00:00Z",
            "state": "new",
            "applied_at": None,
        }
        row.update(overrides)
        return row

    def insert_action(self, **overrides):
        row = self.action(**overrides)
        columns = ", ".join(row)
        marks = ", ".join("?" for _ in row)
        return f"INSERT INTO agg_actions ({columns}) VALUES ({marks})", tuple(
            row.values()
        )

    def test_action_round_trip(self):
        self.ok(*self.insert_action())
        self.ok(
            *self.insert_action(
                id="da-2", state="applied", applied_at="2026-11-25T01:00:00Z"
            )
        )

    def test_action_kind_and_verb_contract(self):
        self.refused(*self.insert_action(kind="bogus"))
        self.refused(*self.insert_action(kind="session", verb="acknowledge"))

    def test_action_payload_must_be_a_json_object(self):
        self.refused(*self.insert_action(payload="[]"))
        self.refused(*self.insert_action(payload='"text"'))
        self.refused(*self.insert_action(payload="not json"))

    def test_action_author_contract(self):
        self.refused(*self.insert_action(author='{"kind":"drew","id":"drew"}'))
        self.refused(
            *self.insert_action(author='{"kind":"nobody","id":"x","label":"X"}')
        )

    def test_action_client_token_unique_only_when_present(self):
        self.ok(*self.insert_action(client_token="ct-1"))
        self.refused(*self.insert_action(id="da-2", client_token="ct-1"))
        self.ok(*self.insert_action(id="da-3"))
        self.ok(*self.insert_action(id="da-4"))

    def test_action_result_timestamp_rule(self):
        self.refused(*self.insert_action(state="applied"))
        self.refused(
            *self.insert_action(state="new", applied_at="2026-11-25T01:00:00Z")
        )

    def test_action_cannot_supersede_itself(self):
        self.refused(*self.insert_action(supersedes_action_id="da-1"))

    def event(self, **overrides):
        row = {
            "id": "ev-1",
            "event_at": "2026-11-25T00:00:00Z",
            "actor": "run-1",
            "kind": "note",
            "ref": "orch:item-1",
            "summary": "hello",
            "prev_hash": HASH,
            "hash": "b" * 64,
            "action_id": None,
            "idempotency_key": None,
        }
        row.update(overrides)
        return row

    def insert_event(self, **overrides):
        row = self.event(**overrides)
        columns = ", ".join(row)
        marks = ", ".join("?" for _ in row)
        return f"INSERT INTO shared_events ({columns}) VALUES ({marks})", tuple(
            row.values()
        )

    def test_event_defaults_and_hash_shape(self):
        self.ok(*self.insert_event())
        row = self.db.execute(
            "SELECT ingested_at, attribution FROM shared_events WHERE id = 'ev-1'"
        ).fetchone()
        self.assertIsNotNone(row[0])
        self.assertEqual(json.loads(row[1])["asserted"], False)
        self.refused(*self.insert_event(id="ev-2", prev_hash="xyz"))
        self.refused(*self.insert_event(id="ev-3", prev_hash="A" * 64))

    def test_event_action_link_rule(self):
        self.ok(*self.insert_action())
        self.ok(
            *self.insert_event(
                kind="action-applied",
                action_id="da-1",
                idempotency_key="action-result:da-1",
            )
        )
        self.refused(
            *self.insert_event(
                id="ev-2",
                kind="action-applied",
                action_id="da-1",
                idempotency_key="action-result:da-1",
            )
        )
        self.refused(
            *self.insert_event(
                id="ev-3",
                kind="action-applied",
                action_id="da-1",
                idempotency_key="wrong-key",
            )
        )
        # Unlinked outcomes (the existing JSON rows' shape) stay valid.
        self.ok(*self.insert_event(id="ev-4", kind="action-rejected"))

    def test_event_attribution_contract(self):
        self.refused(
            *self.insert_event(
                attribution='{"confidence":"authenticated","asserted":false}'
            )
        )
        self.ok(
            *self.insert_event(
                attribution='{"confidence":"authenticated","asserted":true}'
            )
        )

    def test_orch_run_identity_check(self):
        doc = '{"runId":"r1","version":1}'
        self.ok(
            "INSERT INTO orch_runs VALUES ('r1', '2026-11-25T00:00:00Z', 1, '{}', ?, 0)",
            (doc,),
        )
        self.refused(
            "INSERT INTO orch_runs VALUES ('r2', '2026-11-25T00:00:00Z', 1, '{}', '{\"runId\":\"other\",\"version\":1}', 0)"
        )
        # The #8857 case: a document with no runId must not slip
        # through on a NULL check result.
        self.refused(
            "INSERT INTO orch_runs VALUES ('r3', '2026-11-25T00:00:00Z', 1, '{}', '{\"version\":1}', 0)"
        )

    def test_orch_item_identity_check(self):
        self.ok(
            "INSERT INTO orch_runs VALUES ('r1', '2026-11-25T00:00:00Z', 1, '{}', '{\"runId\":\"r1\",\"version\":1}', 1)"
        )
        insert = (
            "INSERT INTO orch_items VALUES ('i1', 'needsDrew', 'decision',"
            " 't', 'test', NULL, 'r', NULL, '2026-11-25T00:00:00Z',"
            " '2026-11-25T00:00:00Z', ?, 'r1', 'r1')"
        )
        self.ok(insert, ('{"id":"i1"}',))
        self.refused(
            "INSERT INTO orch_items VALUES ('i2', 'needsDrew', 'decision',"
            " 't', 'test', NULL, 'r', NULL, '2026-11-25T00:00:00Z',"
            " '2026-11-25T00:00:00Z', '{}', 'r1', 'r1')"
        )

    def test_usage_value_shape(self):
        insert = (
            "INSERT INTO agg_usage_snapshots (snapshot_key, ts, service_id,"
            " metric_id, unit, source, value, observed_by)"
            " VALUES (?, '2026-11-25T00:00:00Z', 'svc', 'm', 'count', 'test', ?, 'test')"
        )
        self.ok(insert, ("k1", "42"))
        self.ok(insert, ("k2", '{"low":1,"high":2}'))
        # A missing 'high' key passes in PostgreSQL too: the check
        # evaluates to NULL there, and a CHECK passes on NULL in both
        # engines. The translation keeps that semantics rather than
        # tightening a contract the PostgreSQL suite never asserted.
        self.ok(insert, ("k3", '{"low":1}'))
        self.refused(insert, ("k4", '"text"'))
        self.refused(insert, ("k5", "[1,2]"))

    def test_ingestion_seen_ids_default_is_an_empty_json_array(self):
        self.ok("INSERT INTO agg_activity_ingestion (source) VALUES ('s1')")
        row = self.db.execute(
            "SELECT seen_ids FROM agg_activity_ingestion WHERE source = 's1'"
        ).fetchone()
        self.assertEqual(row[0], "[]")

    def test_comment_github_ref_shape(self):
        insert = (
            "INSERT INTO shared_comments (id, object_ref, kind, author,"
            " body, created_at, github_state, github_ref)"
            " VALUES (?, 'obj', 'github', ?, 'b', '2026-11-25T00:00:00Z', 'pending', ?)"
        )
        self.ok(insert, ("c1", AUTHOR, "developing-today/code#123"))
        self.ok(insert, ("c2", AUTHOR, "a/b#007"))
        for bad in (
            "noslash#1",
            "a/b#",
            "a/#1",
            "/b#1",
            "a/b#x",
            "a/b/c#1",
            "a/b#1#2",
            "a b/c#1",
        ):
            self.refused(insert, (f"c-{bad}", AUTHOR, bad))

    def test_comment_pending_github_needs_a_ref(self):
        self.refused(
            "INSERT INTO shared_comments (id, object_ref, kind, author,"
            " body, created_at, github_state)"
            " VALUES ('c9', 'obj', 'github', ?, 'b',"
            " '2026-11-25T00:00:00Z', 'pending')",
            (AUTHOR,),
        )


class FailureTests(unittest.TestCase):
    def test_broken_migration_stops_the_run_and_is_not_recorded(self):
        with tempfile.TemporaryDirectory() as tmp:
            migrations_dir = Path(tmp) / "migrations"
            migrations_dir.mkdir()
            for mid, sql in tm.load_migrations():
                (migrations_dir / f"{mid}.up.sql").write_text(sql)
            (migrations_dir / "99999999_0001_broken.up.sql").write_text(
                "CREATE TABLE broken_partial (id text);\n"
                "CREATE TABLE broken_partial (id text);\n"
            )
            client = tm.LocalSqliteClient(str(Path(tmp) / "store.db"))
            verdict = tm.apply_migrations(client, tm.load_migrations(migrations_dir))
            self.assertEqual(verdict["applied"], MIGRATION_IDS)
            self.assertEqual(verdict["failed"]["migration"], "99999999_0001_broken")
            rows = client.query(f"SELECT migration_id FROM {tm.LEDGER_TABLE}")
            self.assertNotIn(
                "99999999_0001_broken", {row["migration_id"] for row in rows}
            )
            # The failed migration's first statement rolled back too:
            # the migration lands whole or not at all.
            tables = {
                row["name"]
                for row in client.query(
                    "SELECT name FROM sqlite_master WHERE type = 'table'"
                )
            }
            self.assertNotIn("broken_partial", tables)
            # A re-run retries the broken migration and fails again,
            # with the good set reported as already applied.
            verdict = tm.apply_migrations(client, tm.load_migrations(migrations_dir))
            self.assertEqual(verdict["applied"], [])
            self.assertEqual(verdict["already_applied"], MIGRATION_IDS)
            self.assertEqual(verdict["failed"]["migration"], "99999999_0001_broken")


# ---------------------------------------------------------------------
# A fake Hrana server: the /v2/pipeline batch semantics the client
# relies on (ok-chained steps; a skipped step is not "ok" for its
# dependents, so the conditional ROLLBACK fires after a failed COMMIT
# chain), backed by a real SQLite database.
# ---------------------------------------------------------------------


def hrana_value(value):
    if value is None:
        return {"type": "null"}
    if isinstance(value, bool):
        return {"type": "integer", "value": str(int(value))}
    if isinstance(value, int):
        return {"type": "integer", "value": str(value)}
    if isinstance(value, float):
        return {"type": "float", "value": value}
    return {"type": "text", "value": str(value)}


class FakeHrana:
    def __init__(self):
        # Handler threads execute the statements; the lock serializes
        # them, so the connection may cross threads.
        self.db = sqlite3.connect(
            ":memory:", isolation_level=None, check_same_thread=False
        )
        self.lock = threading.Lock()

    def step_ran_ok(self, results, errors, index):
        return errors[index] is None and results[index] is not None

    def condition_holds(self, condition, results, errors):
        if condition is None:
            return True
        if condition["type"] == "ok":
            return self.step_ran_ok(results, errors, condition["step"])
        if condition["type"] == "not":
            return not self.condition_holds(condition["cond"], results, errors)
        raise ValueError(f"unknown condition {condition}")

    def batch(self, steps):
        results, errors = [], []
        with self.lock:
            for step in steps:
                if not self.condition_holds(step.get("condition"), results, errors):
                    results.append(None)
                    errors.append(None)
                    continue
                sql = step["stmt"]["sql"]
                try:
                    cursor = self.db.execute(sql)
                    columns = [d[0] for d in cursor.description or []]
                    rows = [[hrana_value(v) for v in row] for row in cursor.fetchall()]
                    results.append(
                        {
                            "cols": [{"name": c} for c in columns],
                            "rows": rows,
                        }
                    )
                    errors.append(None)
                except sqlite3.Error as exc:
                    results.append(None)
                    errors.append({"message": str(exc)})
        return results, errors


def serve_fake_hrana(fake):
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            length = int(self.headers["Content-Length"])
            body = json.loads(self.rfile.read(length))
            steps = body["requests"][0]["batch"]["steps"]
            results, errors = fake.batch(steps)
            payload = {
                "results": [
                    {
                        "type": "ok",
                        "response": {
                            "type": "batch",
                            "result": {
                                "step_results": results,
                                "step_errors": errors,
                            },
                        },
                    }
                ]
            }
            data = json.dumps(payload).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


class HttpClientTests(unittest.TestCase):
    def setUp(self):
        # The client honours ambient proxy settings (as the house
        # Hrana client does); a loopback fake must not be routed
        # through one, so the proxy environment is scrubbed here.
        env = {
            key: value
            for key, value in os.environ.items()
            if "proxy" not in key.lower()
        }
        env["NO_PROXY"] = "127.0.0.1"
        patcher = unittest.mock.patch.dict(os.environ, env, clear=True)
        patcher.start()
        self.addCleanup(patcher.stop)

    def http_client(self, url, token="test-token", host="127.0.0.1"):
        # The pipeline code path runs for real; only the origin pin is
        # bypassed, because the fake server is plain HTTP on loopback
        # and the pin (tested separately below) requires HTTPS.
        client = tm.HttpTursoClient.__new__(tm.HttpTursoClient)
        client.url = url
        client.token = token
        return client

    def test_origin_pin_refusals(self):
        with self.assertRaises(tm.StoreError):
            tm.HttpTursoClient("http://example.com", "t", "example.com")
        with self.assertRaises(tm.StoreError):
            tm.HttpTursoClient("https://other.example.com", "t", "example.com")
        with self.assertRaises(tm.StoreError):
            tm.HttpTursoClient("https://example.com/some/path", "t", "example.com")
        with self.assertRaises(tm.StoreError):
            tm.HttpTursoClient("https://example.com", "", "example.com")

    def test_apply_over_http(self):
        fake = FakeHrana()
        server = serve_fake_hrana(fake)
        self.addCleanup(server.shutdown)
        url = f"http://127.0.0.1:{server.server_address[1]}"
        client = self.http_client(url)
        verdict = tm.apply_migrations(client, tm.load_migrations())
        self.assertIsNone(verdict["failed"])
        self.assertEqual(verdict["applied"], MIGRATION_IDS)
        verdict = tm.apply_migrations(client, tm.load_migrations())
        self.assertEqual(verdict["applied"], [])
        self.assertEqual(verdict["already_applied"], MIGRATION_IDS)
        rows = client.query("SELECT count(*) AS n FROM agg_actions")
        self.assertEqual(rows[0]["n"], "0")

    def test_failed_migration_rolls_back_over_http(self):
        fake = FakeHrana()
        server = serve_fake_hrana(fake)
        self.addCleanup(server.shutdown)
        url = f"http://127.0.0.1:{server.server_address[1]}"
        client = self.http_client(url)
        migrations = tm.load_migrations() + [
            (
                "99999999_0001_broken",
                "CREATE TABLE broken_partial (id text);\n"
                "CREATE TABLE broken_partial (id text);\n",
            )
        ]
        verdict = tm.apply_migrations(client, migrations)
        self.assertEqual(verdict["failed"]["migration"], "99999999_0001_broken")
        tables = {
            row["name"]
            for row in client.query(
                "SELECT name FROM sqlite_master WHERE type = 'table'"
            )
        }
        self.assertNotIn("broken_partial", tables)


class CliTests(unittest.TestCase):
    def test_usage_errors_exit_2(self):
        for argv in (
            [],
            ["--url", "https://example.com"],
            ["--local", "x.db", "--store", "fleet-platform"],
            ["--bogus"],
        ):
            with self.assertRaises(SystemExit) as caught:
                cli.parse_args(argv)
            self.assertEqual(caught.exception.code, 2)

    def test_local_apply_and_rerun(self):
        import io
        from contextlib import redirect_stdout

        with tempfile.TemporaryDirectory() as tmp:
            path = str(Path(tmp) / "store.db")
            with redirect_stdout(io.StringIO()):
                self.assertEqual(cli.main(["--local", path]), 0)
                self.assertEqual(cli.main(["--local", path]), 0)

    def test_store_without_token_fails_before_network(self):
        with tempfile.TemporaryDirectory() as tmp:
            stores = Path(tmp) / "stores.json"
            stores.write_text(
                json.dumps(
                    {
                        "stores": [
                            {
                                "name": "fleet-platform",
                                "address": "https://fleet-platform-dezren39.aws-us-east-1.turso.io",
                            }
                        ]
                    }
                )
            )
            env = dict(os.environ)
            env.pop("STORE_TOKEN", None)
            with unittest.mock.patch.dict(os.environ, env, clear=True):
                code = cli.main(
                    [
                        "--store",
                        "fleet-platform",
                        "--stores-file",
                        str(stores),
                    ]
                )
            self.assertEqual(code, 1)

    def test_unknown_store_is_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            stores = Path(tmp) / "stores.json"
            stores.write_text(json.dumps({"stores": []}))
            code = cli.main(["--store", "nope", "--stores-file", str(stores)])
            self.assertEqual(code, 1)

    def test_dry_run_reports_the_whole_set(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = str(Path(tmp) / "store.db")
            import io
            from contextlib import redirect_stdout

            out = io.StringIO()
            with redirect_stdout(out):
                code = cli.main(["--local", path, "--dry-run"])
            self.assertEqual(code, 0)
            verdict = json.loads(out.getvalue())
            self.assertEqual(verdict["would_apply"], MIGRATION_IDS)
            self.assertFalse(Path(path).exists())


if __name__ == "__main__":
    unittest.main()
