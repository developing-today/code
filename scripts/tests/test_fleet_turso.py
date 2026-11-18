"""Oracle: Hrana typed values and fail-closed atomic batch structure."""
import sys
from pathlib import Path
import json
import unittest
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from fleet_turso import Turso, argument

class Response:
    def __enter__(self): return self
    def __exit__(self,*args): pass
    def read(self): return json.dumps({'results':[{'type':'ok','response':{'result':{'step_errors':[None]*5,'step_results':[None]*5}}}]}).encode()

class Tests(unittest.TestCase):
    def test_types_and_rows(self):
        self.assertEqual(argument(None), {'type':'null'})
        self.assertEqual(argument(5), {'type':'integer','value':'5'})
        self.assertEqual(argument({'x':1})['type'],'text')
        self.assertEqual(Turso.rows({'cols':[{'name':'n'}],'rows':[[{'type':'integer','value':'5'}]]}),[{'n':5}])
    def test_atomic_batch(self):
        db=Turso.__new__(Turso);db.url='https://example.invalid';db.token='test'
        with patch('urllib.request.urlopen',return_value=Response()) as request:
            db.batch([('INSERT INTO t VALUES (?)',[1]),('INSERT INTO t VALUES (?)',[2])],transaction=True)
        steps=json.loads(request.call_args.args[0].data)['requests'][0]['batch']['steps']
        self.assertEqual(steps[0]['stmt']['sql'],'BEGIN IMMEDIATE')
        self.assertEqual(steps[2]['condition'],{'type':'ok','step':1})
        self.assertEqual(steps[3]['stmt']['sql'],'COMMIT')
        self.assertEqual(steps[3]['condition'],{'type':'ok','step':2})
        self.assertEqual(steps[4]['stmt']['sql'],'ROLLBACK')
        self.assertEqual(steps[4]['condition'],{'type':'not','cond':{'type':'ok','step':2}})
if __name__=='__main__':unittest.main()
