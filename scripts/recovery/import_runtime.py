#!/usr/bin/env python3
"""Import audited runtime candidate into fleet-platform. Default read-only dry run.
Usage: import_runtime.py CANDIDATE_DIR --expected-missing-parents N [--apply]
The only existing writable table is ainventory_ingest_batches, missing keys only.
All other candidate tables must be absent. Uses one transaction; no automatic
retry on uncertain writes. No secret/row values printed. Requires staged review.
"""
import argparse,json,sqlite3,subprocess,urllib.request,urllib.error,hashlib
from pathlib import Path
from pg_copy_archive import quote
from prepare_runtime import NAMES,target
PARENT='ainventory_ingest_batches'
class NoRedirect(urllib.request.HTTPRedirectHandler):
 def redirect_request(self,req,fp,code,msg,headers,newurl):return None
OPEN=urllib.request.build_opener(NoRedirect())
def filehash(p):
 with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def arg(v):
 if v is None:return {'type':'null'}
 if isinstance(v,int):return {'type':'integer','value':str(v)}
 if isinstance(v,float):return {'type':'float','value':v}
 return {'type':'text','value':v}
def statements(s):
 buf=''
 for line in s.splitlines(True):
  buf+=line
  if sqlite3.complete_statement(buf):yield buf;buf=''
 if buf.strip():raise ValueError('incomplete schema statement')
def batches(rows,max_rows=500,max_bytes=2000000):
 batch=[];size=0
 for row in rows:
  n=len(json.dumps([arg(v) for v in row]).encode())
  if n>max_bytes:raise ValueError('individual row exceeds safe batch bound')
  if batch and (len(batch)>=max_rows or size+n>max_bytes):yield batch;batch=[];size=0
  batch.append(row);size+=n
 if batch:yield batch
class Client:
 def __init__(self,token,name='fleet-platform'):
  self.token=token;self.baton=None;self.transaction=False;self.host=name+'-dezren39.aws-us-east-1.turso.io';self.url='https://'+self.host+'/v2/pipeline'
 def execute(self,sql,values=()):
  body={'requests':[{'type':'execute','stmt':{'sql':sql,'args':[arg(v) for v in values],'want_rows':True}}]}
  if self.baton:body['baton']=self.baton
  req=urllib.request.Request(self.url,data=json.dumps(body).encode(),headers={'Authorization':'Bearer '+self.token,'Content-Type':'application/json'})
  try:
   with OPEN.open(req,timeout=90) as r:out=json.load(r)
  except urllib.error.HTTPError as e:raise RuntimeError('HTTP '+str(e.code)+'; outcome may be uncertain') from None
  if not isinstance(out,dict) or not isinstance(out.get('results'),list) or len(out['results'])!=1:raise RuntimeError('invalid response shape')
  self.baton=out.get('baton');base=out.get('base_url')
  if base:
   from urllib.parse import urlsplit
   u=urlsplit(base)
   if u.scheme!='https' or u.hostname!=self.host or u.username or u.password or u.port not in (None,443) or u.query or u.fragment or u.path not in ('','/'):raise RuntimeError('unexpected service redirect')
   self.url=base.rstrip('/')+'/v2/pipeline'
  result=out['results'][0]
  if result['type']!='ok':raise RuntimeError('SQL error '+str(result.get('error',{}).get('code','unknown')))
  response=result.get('response',{})
  if response.get('type')!='execute' or not isinstance(response.get('result'),dict) or not isinstance(response['result'].get('rows'),list):raise RuntimeError('invalid execute response')
  if sql=='BEGIN IMMEDIATE':self.transaction=True
  if self.transaction and sql not in ('COMMIT','ROLLBACK') and (not isinstance(self.baton,str) or not self.baton):raise RuntimeError('transaction baton lost')
  if sql in ('COMMIT','ROLLBACK'):self.transaction=False
  rows=response['result']['rows']
  if any(not isinstance(row,list) or any(not isinstance(c,dict) or c.get('type') not in ('null','integer','float','text','blob') for c in row) for row in rows):raise RuntimeError('invalid row shape')
  return [[c.get('value') for c in row] for row in rows]
 def close(self):
  if not self.baton:return
  req=urllib.request.Request(self.url,data=json.dumps({'baton':self.baton,'requests':[{'type':'close'}]}).encode(),headers={'Authorization':'Bearer '+self.token,'Content-Type':'application/json'})
  with OPEN.open(req,timeout=20) as r:r.read()
  self.baton=None

def plan_import(db,client,expected):
 tables=[r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")]
 if set(tables)!={target(n) for n in NAMES}:raise ValueError('unexpected candidate table set')
 existing={r[0] for r in client.execute("SELECT name FROM sqlite_master WHERE type='table'")}
 if PARENT not in existing:raise ValueError('live parent table missing')
 if (set(tables)-{PARENT})&existing:raise ValueError('new target table already exists; refuse partial/duplicate import')
 live_ids={r[0] for r in client.execute('SELECT id FROM '+PARENT)}
 parent_cols=[r[1] for r in db.execute('PRAGMA table_info('+PARENT+')')];idpos=parent_cols.index('id')
 parents=[r for r in db.execute('SELECT * FROM '+PARENT) if r[idpos] not in live_ids]
 if len(parents)!=expected:raise ValueError('missing parent count changed; review required')
 return tables,parents,len(live_ids)

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('candidate',type=Path);p.add_argument('--expected-missing-parents',type=int,required=True);p.add_argument('--apply',action='store_true');p.add_argument('--target',choices=['fleet-platform','fleet-supabase-runtime-tx-stage-20261118','fleet-supabase-runtime-legacy-stage-20261118'],default='fleet-platform');p.add_argument('--expect-db-sha256');p.add_argument('--expect-schema-sha256');a=p.parse_args()
 if a.expected_missing_parents<0:p.error('expected count must be nonnegative')
 hashes={'db':filehash(a.candidate/'runtime.db'),'schema':filehash(a.candidate/'runtime-schema.sql')}
 if a.apply and (hashes['db']!=a.expect_db_sha256 or hashes['schema']!=a.expect_schema_sha256):p.error('reviewed candidate hashes required and must match')
 db=sqlite3.connect('file:'+str(a.candidate/'runtime.db')+'?mode=ro',uri=True)
 if db.execute('PRAGMA integrity_check').fetchone()!=('ok',):raise ValueError('candidate integrity failure')
 if db.execute('PRAGMA foreign_key_check').fetchall():raise ValueError('candidate foreign keys fail')
 ddl=list(statements((a.candidate/'runtime-schema.sql').read_text()))
 token=subprocess.run(['fleet','secrets','get','turso.fleet-platform-rw-token'],capture_output=True,text=True,check=True,timeout=60).stdout.strip().splitlines()[-1]
 if a.target!='fleet-platform':token=subprocess.run(['turso','db','tokens','create',a.target,'--expiration','1d'],capture_output=True,text=True,check=True,timeout=60).stdout.strip()
 c=Client(token,a.target);begun=False;committed=False;commit_attempted=False
 try:
  if a.apply:
   c.execute('PRAGMA foreign_keys=ON')
   if c.execute('PRAGMA foreign_keys')!=[['1']]:raise RuntimeError('foreign keys not enabled')
   c.execute('BEGIN IMMEDIATE');begun=True
  tables,parents,initial_parent_count=plan_import(db,c,a.expected_missing_parents)
  counts={t:db.execute('SELECT count(*) FROM '+quote(t)).fetchone()[0] for t in tables};counts[PARENT]=len(parents)
  print(json.dumps({'mode':'apply' if a.apply else 'dry-run','new_tables':len(tables)-1,'rows_to_insert':counts,'parent_initial_count':initial_parent_count,'candidate_hashes':hashes}),flush=True)
  if not a.apply:return
  if c.execute('SELECT id FROM ainventory_ingest_batches GROUP BY id HAVING id IS NULL OR count(*)>1'):raise RuntimeError('parent IDs not unique/non-null')
  c.execute('CREATE UNIQUE INDEX recovery_ainventory_ingest_batches_id_uq ON ainventory_ingest_batches(id)')
  for st in ddl:
   # Parent already exists and is intentionally never recreated/modified.
   if st.lstrip().startswith('CREATE TABLE '+PARENT+' '):continue
   if ' ON '+PARENT+' ' in st:continue
   c.execute(st)
  for t in [PARENT]+[x for x in tables if x!=PARENT]:
   cols=[r[1] for r in db.execute('PRAGMA table_info('+quote(t)+')')]
   rows=parents if t==PARENT else db.execute('SELECT * FROM '+quote(t)).fetchall()
   for batch in batches(rows):
    sql='INSERT INTO '+quote(t)+' ('+','.join(quote(x) for x in cols)+') VALUES '+','.join('('+','.join('?' for _ in cols)+')' for _ in batch)
    try:c.execute(sql,[v for row in batch for v in row])
    except Exception as e:raise RuntimeError('insert failed table='+t+' batch_rows='+str(len(batch))+' '+type(e).__name__) from e
   actual=int(c.execute('SELECT count(*) FROM '+quote(t))[0][0]);expected=initial_parent_count+len(parents) if t==PARENT else counts[t]
   if actual!=expected:raise RuntimeError('post-insert count mismatch for '+t)
  if c.execute('PRAGMA foreign_key_check'):raise RuntimeError('destination foreign keys fail')
  if filehash(a.candidate/'runtime.db')!=hashes['db'] or filehash(a.candidate/'runtime-schema.sql')!=hashes['schema']:raise RuntimeError('candidate changed during import')
  commit_attempted=True
  c.execute('COMMIT');committed=True
  print(json.dumps({'committed':True,'inserted_rows':sum(counts.values()),'preserved_existing_parents':initial_parent_count}),flush=True)
 except BaseException:
  if commit_attempted and not committed:print('COMMIT outcome uncertain: do not retry; reconcile all destination counts and values',flush=True)
  if begun and not committed and not commit_attempted:
   try:c.execute('ROLLBACK');print('Rollback acknowledged',flush=True)
   except Exception:print('Rollback unverified; inspect destination before retry',flush=True)
  raise
 finally:
  try:c.close()
  except Exception:pass
  db.close()
if __name__=='__main__':main()
