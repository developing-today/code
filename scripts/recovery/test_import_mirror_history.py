import unittest, sqlite3
from unittest.mock import patch
import import_mirror_history as m


class Local:
    def __init__(self):
        self.db = sqlite3.connect(":memory:", isolation_level=None)

    def execute(self, s, args=()):
        return [list(x) for x in self.db.execute(s, args).fetchall()]


class Tests(unittest.TestCase):
    def data(self):
        return {
            t: {
                "columns": [r[1] for r in m.canonical(t)],
                "rows": [["id", '{"x":1}', "hash", None, "2026-11-17"]],
            }
            for t in m.TABLES
        }

    def test_exact_atomic_insert_and_existing_schema(self):
        c = Local()
        c.execute("create table unrelated(id text)")
        c.execute("insert into unrelated values ('safe')")
        d = self.data()
        with patch.dict(m.TABLES, {t: 1 for t in m.TABLES}, clear=True):
            m.apply(c, d)
            m.verify(c, d)
        self.assertEqual(c.execute("select * from unrelated"), [["safe"]])
        with self.assertRaises(ValueError):
            m.preflight(c, d)

    def test_failure_rolls_back_ddl_and_rows(self):
        c = Local()
        d = self.data()
        d[next(reversed(d))]["rows"][0][0] = None
        with (
            patch.dict(m.TABLES, {t: 1 for t in m.TABLES}, clear=True),
            self.assertRaises(sqlite3.IntegrityError),
        ):
            m.apply(c, d)
        self.assertEqual(
            c.execute("select name from sqlite_master where type='table'"), []
        )

    def test_exact_multiplicity_and_null(self):
        self.assertNotEqual(m.digest_rows([["a"]]), m.digest_rows([["a"], ["a"]]))
        self.assertNotEqual(m.digest_rows([[None]]), m.digest_rows([["None"]]))

    def test_schema_rejects_extra_trigger(self):
        c = Local()
        t = next(iter(m.TABLES))
        c.execute(m.ddls()[t])
        c.execute("create trigger bad before insert on " + t + " begin select 1; end")
        with self.assertRaises(ValueError):
            m.preflight(c, self.data())


if __name__ == "__main__":
    unittest.main()
