import unittest,copy
from import_aa_missing import validate,preflight,EXPECTED
class Fake:
 def __init__(self,data,collision=False):self.data=data;self.collision=collision
 def execute(self,s,args=()):
  t=next(t for t in self.data if t in s);d=self.data[t]
  if s.startswith('PRAGMA'):return [[str(i),col] for i,col in enumerate(d['columns'])]
  if 'count(*)' in s:return [[str(d['expected_existing_count'])]]
  return [['1']] if self.collision else []
class AATests(unittest.TestCase):
 def setUp(self):self.d={t:{'columns':['id','data'],'rows':[[i,'{}'] for i in range(n)],'allowed_ids':list(range(n)),'expected_existing_count':10} for t,n in EXPECTED.items()}
 def test_valid(self):validate(self.d);preflight(Fake(self.d),self.d)
 def test_collision_rejected(self):
  with self.assertRaises(ValueError):preflight(Fake(self.d,True),self.d)
 def test_unapproved_ids_rejected(self):
  self.d['ainventory_sources']['rows'][0][0]=999
  with self.assertRaises(ValueError):validate(self.d)
 def test_unknown_table_rejected(self):
  self.d['extra']={}
  with self.assertRaises(ValueError):validate(self.d)
if __name__=='__main__':unittest.main()
