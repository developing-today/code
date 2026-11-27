"""Oracle: Hrana typed values and fail-closed atomic batch structure."""

import sys
from pathlib import Path
import json
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fleet_turso import Turso, argument


class Response:
    def __enter__(self):
        return self

    def __exit__(self, *args):
        pass

    def read(self):
        return json.dumps(
            {
                "results": [
                    {
                        "type": "ok",
                        "response": {
                            "type": "batch",
                            "result": {
                                "step_errors": [None] * 5,
                                "step_results": [{"cols": [], "rows": []}] * 4 + [None],
                            },
                        },
                    }
                ]
            }
        ).encode()


class Tests(unittest.TestCase):
    def test_types_and_rows(self):
        self.assertEqual(argument(None), {"type": "null"})
        self.assertEqual(argument(5), {"type": "integer", "value": "5"})
        self.assertEqual(argument({"x": 1})["type"], "text")
        self.assertEqual(
            Turso.rows(
                {"cols": [{"name": "n"}], "rows": [[{"type": "integer", "value": "5"}]]}
            ),
            [{"n": 5}],
        )

    def test_atomic_batch(self):
        db = Turso.__new__(Turso)
        db.url = "https://example.invalid"
        db.token = "test"
        with patch("fleet_turso.open_request", return_value=Response()) as request:
            db.batch(
                [("INSERT INTO t VALUES (?)", [1]), ("INSERT INTO t VALUES (?)", [2])],
                transaction=True,
            )
        steps = json.loads(request.call_args.args[0].data)["requests"][0]["batch"][
            "steps"
        ]
        self.assertEqual(steps[0]["stmt"]["sql"], "BEGIN IMMEDIATE")
        self.assertEqual(steps[2]["condition"], {"type": "ok", "step": 1})
        self.assertEqual(steps[3]["stmt"]["sql"], "COMMIT")
        self.assertEqual(steps[3]["condition"], {"type": "ok", "step": 2})
        self.assertEqual(steps[4]["stmt"]["sql"], "ROLLBACK")
        self.assertEqual(
            steps[4]["condition"], {"type": "not", "cond": {"type": "ok", "step": 3}}
        )

    def test_destination_rejected_before_secret_lookup(self):
        for url in (
            "https://other.turso.io",
            "https://fleet-platform-dezren39.aws-us-east-1.turso.io",
            "https://d1-gateway.developing-today.workers.dev@evil.example/db/fleet-platform",
            "https://d1-gateway.developing-today.workers.dev/db/fleet-queue",
        ):
            with (
                patch.dict("os.environ", {"FLEET_TURSO_URL": url}),
                patch("subprocess.run") as fetch,
            ):
                with self.assertRaises(ValueError):
                    Turso()
                fetch.assert_not_called()

    def test_ambiguous_transport_does_not_claim_rollback(self):
        db = Turso.__new__(Turso)
        db.url = "https://example.invalid"
        db.token = "test"
        with patch("fleet_turso.open_request", side_effect=TimeoutError()):
            with self.assertRaisesRegex(RuntimeError, "outcome unverified"):
                db.batch([("INSERT INTO t VALUES (?)", [1])], transaction=True)

    def test_commit_error_rollback_proof(self):
        db = Turso.__new__(Turso)
        db.url = "https://example.invalid"
        db.token = "test"
        payload = {
            "results": [
                {
                    "type": "ok",
                    "response": {
                        "type": "batch",
                        "result": {
                            "step_errors": [
                                None,
                                None,
                                {"message": "commit failed"},
                                None,
                            ],
                            "step_results": [
                                {"cols": [], "rows": []},
                                {"cols": [], "rows": []},
                                None,
                                {"cols": [], "rows": []},
                            ],
                        },
                    },
                }
            ]
        }

        class FailedCommit(Response):
            def read(self):
                return json.dumps(payload).encode()

        with patch("fleet_turso.open_request", return_value=FailedCommit()):
            with self.assertRaisesRegex(RuntimeError, "rollback confirmed"):
                db.batch([("INSERT INTO t VALUES (?)", [1])], transaction=True)
        payload["results"][0]["response"]["result"]["step_results"][-1] = None
        with patch("fleet_turso.open_request", return_value=FailedCommit()):
            with self.assertRaisesRegex(RuntimeError, "outcome unverified"):
                db.batch([("INSERT INTO t VALUES (?)", [1])], transaction=True)

    def test_missing_success_evidence_is_not_success(self):
        db = Turso.__new__(Turso)
        db.url = "https://example.invalid"
        db.token = "test"
        for results, errors in (
            ([None] * 4, [None] * 4),
            ([], []),
            ([{}, {}, {}, None], [None] * 4),
            ([{"cols": [], "rows": []}] * 2 + [None, None], [None] * 4),
        ):
            payload = {
                "results": [
                    {
                        "type": "ok",
                        "response": {
                            "type": "batch",
                            "result": {"step_errors": errors, "step_results": results},
                        },
                    }
                ]
            }

            class Incomplete(Response):
                def read(self):
                    return json.dumps(payload).encode()

            with patch("fleet_turso.open_request", return_value=Incomplete()):
                with self.assertRaisesRegex(RuntimeError, "outcome unverified"):
                    db.batch([("INSERT INTO t VALUES (?)", [1])], transaction=True)

    def test_redirect_does_not_forward_bearer(self):
        import http.server
        import threading
        import urllib.request
        import urllib.error
        from fleet_turso import open_request

        received = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                received.append(self.path)
                if self.path == "/start":
                    self.send_response(302)
                    self.send_header(
                        "Location", f"http://127.0.0.1:{self.server.server_port}/target"
                    )
                else:
                    self.send_response(200)
                self.end_headers()

            def log_message(self, *args):
                pass

        server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            req = urllib.request.Request(
                f"http://127.0.0.1:{server.server_port}/start",
                headers={"Authorization": "Bearer test"},
            )
            with self.assertRaises(urllib.error.HTTPError) as caught:
                open_request(req, timeout=3)
            self.assertEqual(caught.exception.code, 302)
            self.assertEqual(received, ["/start"])
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == "__main__":
    unittest.main()
