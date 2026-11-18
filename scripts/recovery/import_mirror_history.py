#!/usr/bin/env python3
"""Recover exactly four archived history tables into fleet-mirror, insert-only.
prepare ARCHIVE OUTPUT; run CANDIDATE [--apply --expect-sha256 HASH];
verify CANDIDATE. Stage target optional. No data/credentials printed.
Existing target tables must be empty with exactly canonical schema/no triggers.
"""
import argparse,ast,collections,hashlib,json,sqlite3,subprocess
from pathlib import Path
from functools import lru_cache
from import_runtime import Client,quote,batches,filehash
COUNTS={'agent_agents':941,'goals_goals':299,'goals_updates':1,'scheduler_jobs':2354}
TABLES={'agent_mirror_history_'+k:v for k,v in COUNTS.items()}
ROOT=Path(__file__).resolve().parents[2]
@lru_cache(maxsize=1)
def ddls():
 tree=ast.parse((ROOT/'pkgs/agent-backup/turso_client.py').read_text())
 d=next(ast.literal_eval(n.value) for n in tree.body if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='DDL' for t in n.targets))
 return {t:d[t] for t in TABLES}
@lru_cache(maxsize=4)
def canonical(t):
 c=sqlite3.connect(':memory:');c.execute(ddls()[t]);cols=c.execute('PRAGMA table_info('+quote(t)+')').fetchall();c.close();return [list(r) for r in cols]
def validate(d):
 if set(d)!=set(TABLES):raise ValueError('wrong table set')
 for t,n in TABLES.items():
  x=d[t];cols=[r[1] for r in canonical(t)]
  if x['columns']!=cols or len(x['rows'])!=n:raise ValueError('wrong shape/count')
  for row in x['rows']:
   if len(row)!=len(cols) or any(v is not None and not isinstance(v,str) for v in row):raise ValueError('non-text value')
   if any(row[i] is None and spec[3] for i,spec in enumerate(canonical(t))):raise ValueError('null required value')
def digest_rows(rows):
 # sorted multiset retains duplicate multiplicity and exact original text/NULLs.
 raw=sorted(json.dumps(r,ensure_ascii=False,separators=(',',':')) for r in rows)
 return hashlib.sha256(('\n'.join(raw)+'\n').encode()).hexdigest()
def preflight(c,d):
 for t in TABLES:
  info=c.execute('PRAGMA table_info('+quote(t)+')')
  if info:
   norm=[[int(r[0]),r[1],r[2],int(r[3]),r[4],int(r[5])] for r in info]
   if norm!=canonical(t):raise ValueError('schema mismatch '+t)
   if int(c.execute('SELECT count(*) FROM '+quote(t))[0][0]):raise ValueError('nonempty target '+t)
   extras=c.execute("SELECT type,name,sql FROM sqlite_master WHERE tbl_name=? AND type IN ('trigger','index')",[t])
   expected='CREATE INDEX '+t+'_key_idx ON '+t+' ('+canonical(t)[0][1]+', valid_to)'
   if any(kind!='index' or name!=t+'_key_idx' or sql!=expected for kind,name,sql in extras):raise ValueError('unexpected constraints '+t)
def verify(c,d):
 out={}
 for t,x in d.items():
  rows=c.execute('SELECT '+','.join(map(quote,x['columns']))+' FROM '+quote(t))
  got=digest_rows(rows);want=digest_rows(x['rows'])
  if got!=want or len(rows)!=TABLES[t]:raise ValueError('value mismatch '+t)
  out[t]={'rows':len(rows),'sha256':got,'exact_multiset':True}
 return out
def apply(c,d):
 begun=False;attempt=False
 try:
  c.execute('BEGIN IMMEDIATE');begun=True;preflight(c,d)
  before=c.execute("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name")
  for t,x in d.items():
   c.execute(ddls()[t])
   for batch in batches(x['rows'],max_rows=100):
    c.execute('INSERT INTO '+quote(t)+' ('+','.join(map(quote,x['columns']))+') VALUES '+','.join('('+','.join('?' for _ in x['columns'])+')' for _ in batch),[v for row in batch for v in row])
  report=verify(c,d)
  after=c.execute("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name")
  if not all(row in after for row in before):raise ValueError('preexisting schema changed')
  attempt=True;c.execute('COMMIT');return report
 except BaseException:
  if attempt:print('COMMIT outcome uncertain; reconcile before retry',flush=True)
  elif begun:
   try:c.execute('ROLLBACK');print('Rollback acknowledged',flush=True)
   except Exception:print('Rollback unverified',flush=True)
  raise

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('mode',choices=['prepare','run','verify']);p.add_argument('candidate',type=Path);p.add_argument('--archive',type=Path);p.add_argument('--target',choices=['fleet-mirror','fleet-mirror-recovery-stage-20261118'],default='fleet-mirror');p.add_argument('--apply',action='store_true');p.add_argument('--expect-sha256');p.add_argument('--report',type=Path);a=p.parse_args()
 if a.mode=='prepare':
  if not a.archive:p.error('--archive required')
  c=sqlite3.connect('file:'+str(a.archive)+'?mode=ro',uri=True);d={}
  for t in TABLES:
   cols=[r[1] for r in canonical(t)];src=t.replace('agent_','agent.',1)
   d[t]={'columns':cols,'rows':[list(r) for r in c.execute('SELECT '+','.join(map(quote,cols))+' FROM '+quote(src))]}
  validate(d)
  with a.candidate.open('x') as f:json.dump(d,f,ensure_ascii=False)
  a.candidate.chmod(0o600);print(json.dumps({'candidate_sha256':filehash(a.candidate),'counts':TABLES}));return
 raw=a.candidate.read_bytes();h=hashlib.sha256(raw).hexdigest();d=json.loads(raw);validate(d)
 if a.apply and (a.mode!='run' or h!=a.expect_sha256):p.error('apply requires run and reviewed SHA256')
 if a.target=='fleet-mirror':cmd=['fleet','secrets','get','turso.fleet-mirror-rw-token']
 else:cmd=['turso','db','tokens','create',a.target,'--expiration','1d']
 token=subprocess.run(cmd,capture_output=True,text=True,check=True,timeout=60).stdout.strip();c=Client(token,a.target)
 try:
  if a.mode=='verify':out=verify(c,d)
  elif a.apply:
   apply(c,d);c.close();c=Client(token,a.target);out=verify(c,d)
  else:preflight(c,d);out={'dry_run_counts':TABLES}
  report={'target':a.target,'candidate_sha256':h,'result':out,'fresh_connection_verification':a.apply or a.mode=='verify'}
  if a.report:
   with a.report.open('x') as f:json.dump(report,f,indent=2)
   a.report.chmod(0o600)
  print(json.dumps(report),flush=True)
 finally:c.close()
if __name__=='__main__':main()
