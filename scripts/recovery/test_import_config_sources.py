"""Oracle: SQLite constraint/default behavior and fixed table/count contract."""

import unittest, sqlite3
from import_config_sources import TABLES, ddl, validate


class Tests(unittest.TestCase):
    def data(self):
        return {
            t: [
                [i, "name", "kind", None, None, "batch", "2026-11-01T00:00:00Z", None]
                for i in range(n)
            ]
            for t, n in TABLES.items()
        }

    def test_valid(self):
        validate(self.data())

    def test_wrong_table(self):
        d = self.data()
        d["external_comms"] = d.pop("home_sources")
        with self.assertRaises(ValueError):
            validate(d)

    def test_required(self):
        d = self.data()
        d["home_sources"][0][6] = None
        with self.assertRaises(ValueError):
            validate(d)

    def test_ddl(self):
        c = sqlite3.connect(":memory:")
        c.execute(ddl("home_sources"))
        c.execute(
            "INSERT INTO home_sources(name,kind,import_batch) VALUES ('a','b','c')"
        )
        r = c.execute("SELECT id,imported_at FROM home_sources").fetchone()
        self.assertEqual(r[0], 1)
        self.assertTrue(r[1].endswith("Z"))
        with self.assertRaises(sqlite3.IntegrityError):
            c.execute(
                "INSERT INTO home_sources(id,name,kind,import_batch) VALUES (1,'a','b','c')"
            )


if __name__ == "__main__":
    unittest.main()
