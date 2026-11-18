"""Oracle: immutable identity/content under UPDATE, DELETE and OR REPLACE."""
import pathlib
import sqlite3
import unittest

TABLES=('findings','interpretation_runs','notes','measurement_events','price_events','provider_policies')
class Guards(unittest.TestCase):
    def test_no_mutation_even_without_recursive_triggers(self):
        with sqlite3.connect(':memory:') as db:
            db.execute('PRAGMA recursive_triggers=OFF')
            for t in TABLES:db.execute(f'CREATE TABLE ainventory_{t}(id INTEGER PRIMARY KEY,body TEXT)')
            db.executescript(pathlib.Path(__file__).with_name('ainventory-turso-guards.sql').read_text())
            for t in TABLES:
                db.execute(f'INSERT INTO ainventory_{t}(body) VALUES(?)',['original'])
                for sql in (f'UPDATE ainventory_{t} SET body="changed"',f'DELETE FROM ainventory_{t}',f'INSERT OR REPLACE INTO ainventory_{t} VALUES(1,"changed")'):
                    with self.subTest(table=t,sql=sql),self.assertRaises(sqlite3.IntegrityError):db.execute(sql)
                    self.assertEqual(db.execute(f'SELECT body FROM ainventory_{t}').fetchall(),[('original',)])
        db.close()
if __name__=='__main__':unittest.main()
