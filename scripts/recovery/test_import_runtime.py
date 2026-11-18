import unittest,sqlite3,io,json
from unittest.mock import patch
from import_runtime import plan_import,statements,arg,PARENT,Client,batches
from prepare_runtime import NAMES,target
class FakeClient:
 def __init__(self,extra=None):self.extra=extra
 def execute(self,s):
  if 'sqlite_master' in s:return [[PARENT]]+([[self.extra]] if self.extra else [])
  return [['existing']]
class ImportTests(unittest.TestCase):
 def setUp(self):
  self.db=sqlite3.connect(':memory:')
  for n in NAMES:self.db.execute('CREATE TABLE '+target(n)+' (id TEXT PRIMARY KEY)')
  self.db.executemany('INSERT INTO '+PARENT+' VALUES (?)',[('existing',),('missing',)])
 def tearDown(self):self.db.close()
 def test_missing_only(self):
  tables,parents,n=plan_import(self.db,FakeClient(),1);self.assertEqual(parents,[('missing',)]);self.assertEqual(n,1)
 def test_guard_existing(self):
  with self.assertRaises(ValueError):plan_import(self.db,FakeClient('alerts_incidents'),1)
 def test_guard_count(self):
  with self.assertRaises(ValueError):plan_import(self.db,FakeClient(),2)
 def test_trigger_parser(self):
  s='CREATE TABLE t (id INTEGER);\nCREATE TRIGGER guard BEFORE DELETE ON t BEGIN SELECT RAISE(ABORT,\'no\'); END;\n'
  self.assertEqual(len(list(statements(s))),2)
  with self.assertRaises(ValueError):list(statements('CREATE TABLE unfinished('))
 def test_baton_required(self):
  response={'results':[{'type':'ok','response':{'type':'execute','result':{'rows':[]}}}]}
  with patch('urllib.request.urlopen',return_value=io.BytesIO(json.dumps(response).encode())):
   with self.assertRaisesRegex(RuntimeError,'baton'):Client('test').execute('BEGIN IMMEDIATE')
 def test_redirect_rejected(self):
  response={'base_url':'https://fleet-platform-dezren39.aws-us-east-1.turso.io@evil.example','results':[{'type':'ok','response':{'type':'execute','result':{'rows':[]}}}]}
  with patch('urllib.request.urlopen',return_value=io.BytesIO(json.dumps(response).encode())):
   with self.assertRaisesRegex(RuntimeError,'redirect'):Client('test').execute('SELECT 1')
 def test_batch_bounds(self):
  self.assertEqual([len(x) for x in batches([(1,)]*1001)], [500,500,1])
  with self.assertRaises(ValueError):list(batches([('long string',)],max_bytes=5))
 def test_args(self):self.assertEqual(arg(None),{'type':'null'});self.assertEqual(arg(9),{'type':'integer','value':'9'})
if __name__=='__main__':unittest.main()
