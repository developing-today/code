"""Oracle: DISTINCT ON source ordering and all-or-nothing insert-only state."""
import json,sqlite3,unittest,subprocess,sys,pathlib,tempfile
from unittest.mock import patch
import ainventory_gather as g

class Local:
 def __init__(self):
  self.c=sqlite3.connect(':memory:');self.c.row_factory=sqlite3.Row
  self.c.executescript('''CREATE TABLE ainventory_models(name,provider);
  CREATE TABLE ainventory_ingest_batches(id TEXT PRIMARY KEY, inserted_counts TEXT);
  CREATE TABLE ainventory_measurement_events(id INTEGER PRIMARY KEY,model_name TEXT NOT NULL,model_variant_key,effort,benchmark,benchmark_version,value_num,source_date,ingested_at,raw);
  CREATE TABLE ainventory_price_events(id INTEGER PRIMARY KEY,model_name TEXT NOT NULL,provider,context_min_tokens,context_max_tokens,window_kind,effective_date,source_date,ingested_at,input_usd_per_1m);
  ''')
 def query(self,sql,args=()):return [dict(r) for r in self.c.execute(sql,args)]
 def batch(self,statements,transaction=False):
  assert transaction
  with self.c:
   for sql,args in statements:self.c.execute(sql,[json.dumps(v) if isinstance(v,(dict,list)) else v for v in args])

class Gatherer(unittest.TestCase):
 def setUp(self):self.db=Local()
 def tearDown(self):self.db.c.close()
 def test_view_partition_and_order(self):
  rows=[(1,'m',None,None,'b',None,1,None,'2026-01-03',None),(2,'m',None,'','b','',2,'2026-01-01','2026-01-01',None),(3,'m',None,'','b','',3,'2026-01-01','2026-01-01',None),(4,'m',None,'high','b','',4,'2026-01-01','2026-01-01',None)]
  self.db.c.executemany('INSERT INTO ainventory_measurement_events VALUES(?,?,?,?,?,?,?,?,?,?)',rows)
  self.assertEqual({r['id'] for r in self.db.query(g.CURRENT_MEASUREMENTS)},{3,4})
  self.db.c.executemany('INSERT INTO ainventory_price_events VALUES(?,?,?,?,?,?,?,?,?,?)',[(1,'m',None,0,None,'flat',None,'2026-01-03','2026-01-03',1),(2,'m','',0,None,'flat','2026-01-01','2026-01-01','2026-01-01',2),(3,'m',None,100,None,'flat',None,'2026-01-01','2026-01-01',3)])
  self.assertEqual({r['id'] for r in self.db.query(g.CURRENT_PRICES)},{2,3})
 def test_atomic_json_and_injection_literal(self):
  name="x'); DROP TABLE ainventory_models; --"
  g.persist_events(self.db,{'id':'b','inserted_counts':{'measurement_events':1}},[{'model_name':name,'raw':{'x':1}}],[])
  self.assertEqual(self.db.query('select model_name,raw from ainventory_measurement_events'),[{'model_name':name,'raw':'{"x": 1}'}])
  self.assertEqual(len(self.db.query('select * from ainventory_ingest_batches')),1)
  self.assertEqual(self.db.query('select * from ainventory_models'),[])
 def test_rollback_all_on_bad_event(self):
  with self.assertRaises(sqlite3.IntegrityError):g.persist_events(self.db,{'id':'b','inserted_counts':{}},[{'model_name':'valid'},{'model_name':None}],[])
  self.assertEqual(self.db.query('select * from ainventory_ingest_batches'),[])
  self.assertEqual(self.db.query('select * from ainventory_measurement_events'),[])
 def test_strict_args_before_io(self):
  p=subprocess.run([sys.executable,str(pathlib.Path(g.__file__)),'--wat'],capture_output=True,text=True)
  self.assertEqual(p.returncode,2);self.assertIn('unrecognized arguments',p.stderr)
 def test_read_all_not_truncated(self):
  self.db.c.executemany('insert into ainventory_models values(?,?)',[(str(i),'p') for i in range(1001)])
  self.assertEqual(len(g.read_current(self.db)[0]),1001)
 def test_empty_live_run_never_writes_batch(self):
  with tempfile.TemporaryDirectory() as d, patch.object(sys,'argv',['gather','--live','--out',d]), patch.object(g,'Turso',return_value=self.db), patch.object(g,'fetch',side_effect=['', '{}']), patch.object(self.db,'batch',side_effect=AssertionError('unexpected write')):
   self.assertEqual(g.main(),0)
 def test_live_batch_explicit_timestamps(self):
  # Oracle: legacy deployed parent has no SQL defaults. Both timestamps required.
  known=[{'name':'model','provider':'OpenAI'}]
  aa={'release_name':'model','release_slug':'model','effort':'high','intelligence_index':10,'cost_per_task_usd':None,'output_tokens_per_task':None,'ttft_seconds':None,'output_tps_short':None,'source_ref':'https://example.invalid','price_input_per_1m':None}
  with tempfile.TemporaryDirectory() as d, patch.object(sys,'argv',['gather','--live','--out',d]), patch.object(g,'Turso',return_value=self.db), patch.object(g,'read_current',return_value=(known,[],[],[])), patch.object(g,'fetch',side_effect=['', '{}']), patch.object(g,'parse_aa_models',return_value=[aa]), patch.object(g,'persist_events') as persist:
   self.assertEqual(g.main(),0)
   batch=persist.call_args.args[1]
   self.assertRegex(batch['ran_at'],r'^\d{4}-\d{2}-\d{2}T.*Z$')
   self.assertEqual(batch['created_at'],batch['ran_at'])
   self.assertEqual(batch['inserted_counts'],{'measurement_events':1,'price_events':0})
 def test_numeric_column_name(self):
  sql,args=g.insert_statement('price_events',[{'input_usd_per_1m':1.25}])
  self.assertIn('input_usd_per_1m',sql);self.assertEqual(args,[1.25])
 def test_table_allowlist(self):
  with self.assertRaises(ValueError):g.insert_statement('models',[{'id':1}])

if __name__=='__main__':unittest.main()
