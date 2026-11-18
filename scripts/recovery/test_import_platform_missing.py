"""Oracle: SQLite state, exact key/temporal equivalence, rejected invalid input."""
import unittest,sqlite3,copy
from import_platform_missing import validate,check_absent,preflight
class C:
 def __init__(self):self.db=sqlite3.connect(':memory:');self.db.execute('CREATE TABLE agent_quota_records(source TEXT,id TEXT,ts TEXT,PRIMARY KEY(source,id,ts))')
 def execute(self,s,args=()):return [[str(v) if isinstance(v,int) else v for v in r] for r in self.db.execute(s,args)]
class Tests(unittest.TestCase):
 def setUp(self):
  self.c=C();self.t='agent_quota_records';self.x={'columns':['source','id','ts'],'keys':['source','id','ts'],'temporal_keys':['ts'],'rows':[['a','b','2026-11-01T00:00:00Z']],'schema':self.c.execute('SELECT type,name,tbl_name,sql FROM sqlite_master WHERE tbl_name=? ORDER BY type,name',[self.t]),'initial_count':0}
 def test_missing(self):validate({self.t:self.x});preflight(self.c,{self.t:self.x})
 def test_temporal_collision(self):
  self.c.db.execute("INSERT INTO agent_quota_records VALUES ('a','b','2026-11-01 00:00:00+00')")
  with self.assertRaises(ValueError):check_absent(self.c,self.t,self.x)
 def test_forbidden(self):
  with self.assertRaises(ValueError):validate({'agent_external_comms':self.x})
 def test_duplicate(self):
  self.x['rows']*=2
  with self.assertRaises(ValueError):validate({self.t:self.x})
 def test_null_key(self):
  self.x['rows'][0][0]=None
  with self.assertRaises(ValueError):validate({self.t:self.x})
 def test_count_drift(self):
  self.x['initial_count']=1
  with self.assertRaises(ValueError):preflight(self.c,{self.t:self.x})
 def test_preserves_live_additions(self):
  self.c.db.execute("INSERT INTO agent_quota_records VALUES ('other','x','2026-01-01T00:00:00Z')")
  self.assertEqual(preflight(self.c,{self.t:self.x})[self.t],1)
 def test_schema_drift(self):
  self.x['schema']=[]
  with self.assertRaises(ValueError):preflight(self.c,{self.t:self.x})
 def test_trigger_refusal(self):
  self.x['schema'].append(['trigger','x',self.t,'x'])
  with self.assertRaises(ValueError):validate({self.t:self.x})
 def test_many_keys(self):
  self.x['rows']=[['a',str(i),'2026-11-01T00:00:00Z'] for i in range(1200)];check_absent(self.c,self.t,self.x)
if __name__=='__main__':unittest.main()
