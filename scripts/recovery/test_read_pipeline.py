"""Oracle: protocol shape/baton invariants; malformed or mutating requests fail closed."""

import io, json, unittest
from unittest.mock import patch
from import_runtime import Client
from read_pipeline import execute_reads


def response(n=1, baton="b"):
    return {
        "baton": baton,
        "results": [
            {
                "type": "ok",
                "response": {
                    "type": "execute",
                    "result": {
                        "cols": [{"name": "1"}],
                        "rows": [[{"type": "integer", "value": "1"}]],
                    },
                },
            }
            for _ in range(n)
        ],
    }


class Tests(unittest.TestCase):
    def run_response(self, value, queries=None):
        c = Client("test")
        c.transaction = True
        c.baton = "a"
        with patch(
            "read_pipeline.OPEN.open",
            return_value=io.BytesIO(json.dumps(value).encode()),
        ):
            return execute_reads(c, queries or [("SELECT 1", [])])

    def test_valid(self):
        self.assertEqual(self.run_response(response()), [[["1"]]])

    def test_count(self):
        with self.assertRaises(RuntimeError):
            self.run_response(response(0))

    def test_lost_baton(self):
        with self.assertRaises(RuntimeError):
            self.run_response(response(baton=None))

    def test_redirect(self):
        r = response()
        r["base_url"] = "https://evil.example/"
        with self.assertRaises(RuntimeError):
            self.run_response(r)

    def test_error(self):
        r = response()
        r["results"][0] = {"type": "error"}
        with self.assertRaises(RuntimeError):
            self.run_response(r)

    def test_invalid_cell(self):
        r = response()
        r["results"][0]["response"]["result"]["rows"] = [[{}]]
        with self.assertRaises(RuntimeError):
            self.run_response(r)

    def test_no_mutations(self):
        for sql in [
            "INSERT INTO x VALUES (1)",
            "PRAGMA foreign_keys=OFF",
            "SELECT 1; DELETE FROM x",
        ]:
            with self.assertRaises(ValueError):
                self.run_response(response(), [(sql, [])])

    def test_empty_result_requires_columns(self):
        r = response()
        r["results"][0]["response"]["result"] = {"rows": []}
        with self.assertRaises(RuntimeError):
            self.run_response(r)

    def test_missing_cell_value(self):
        r = response()
        r["results"][0]["response"]["result"]["rows"] = [[{"type": "text"}]]
        with self.assertRaises(RuntimeError):
            self.run_response(r)

    def test_wrong_row_width(self):
        r = response()
        r["results"][0]["response"]["result"]["rows"] = [[]]
        with self.assertRaises(RuntimeError):
            self.run_response(r)

    def test_invalid_integer(self):
        r = response()
        r["results"][0]["response"]["result"]["rows"] = [
            [{"type": "integer", "value": "wrong"}]
        ]
        with self.assertRaises(RuntimeError):
            self.run_response(r)

    def test_chunked_baton(self):
        c = Client("test")
        c.transaction = True
        c.baton = "a"
        calls = []

        def opener(req, timeout):
            b = json.loads(req.data)
            calls.append(b)
            return io.BytesIO(json.dumps(response(len(b["requests"]), "next")).encode())

        with patch("read_pipeline.OPEN.open", side_effect=opener):
            out = execute_reads(c, [("SELECT 1", [])] * 21)
        self.assertEqual(len(out), 21)
        self.assertEqual([len(x["requests"]) for x in calls], [20, 1])
        self.assertEqual(calls[1]["baton"], "next")


if __name__ == "__main__":
    unittest.main()
