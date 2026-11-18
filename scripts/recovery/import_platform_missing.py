#!/usr/bin/env python3
"""Read-only by default. Usage: import_platform_missing.py CANDIDATE
[--apply --expect-sha256 SHA] [--target fleet-platform|fleet-platform-reconcile-stage-20261118].
Strictly INSERT-only historical recovery into 17 allowlisted existing tables.
No credential values or row contents printed. No retry after ambiguous COMMIT.
"""
import argparse,json,sqlite3,subprocess,hashlib,math,datetime
from pathlib import Path
from import_runtime import Client,quote,batches,filehash
TABLES={'agent_mirror_'+s for s in ('agent_agents','compaction_stats_daily','conversation_stats_daily','distincts_daily','goals_updates','histograms_daily','message_token_stats_daily','run_stats_daily','scheduler_job_runs','scheduler_jobs','session_activity_hourly','session_stats_daily','sessions','token_usage_daily','token_usage_hourly')}|{'agent_quota_records','ci_events'}
def canonical_time(v):
 if not isinstance(v,str):raise ValueError('temporal key must be text')
 dt=datetime.datetime.fromisoformat(v.replace('Z','+00:00'))
 if dt.tzinfo is None:raise ValueError('temporal key requires timezone')
 return dt.astimezone(datetime.timezone.utc).isoformat().replace('+00:00','Z')
def validate(data):
 if not data or set(data)-TABLES:raise ValueError('unexpected table')
 for table,x in data.items():
  cols=x['columns'];keys=x['keys']
  if not cols or len(set(cols))!=len(cols) or not keys or not set(keys)<=set(cols) or not set(x['temporal_keys'])<=set(keys):raise ValueError('invalid columns/keys')
  if not isinstance(x['initial_count'],int) or x['initial_count']<0:raise ValueError('invalid count')
  seen=set()
  for row in x['rows']:
   if len(row)!=len(cols) or any(not isinstance(v,(str,int,float,type(None))) or isinstance(v,float) and not math.isfinite(v) for v in row):raise ValueError('invalid row')
   key=tuple(canonical_time(row[cols.index(k)]) if k in x['temporal_keys'] else row[cols.index(k)] for k in keys)
   if any(v is None for v in key) or key in seen:raise ValueError('null/duplicate key')
   seen.add(key)
  if any(s[0]=='trigger' for s in x['schema']):raise ValueError('target triggers not allowed')
def check_absent(c,table,x):
 cols=x['columns'];keys=x['keys'];positions=[cols.index(k) for k in keys]
 lhs=[("julianday(CASE WHEN substr("+quote(k)+",-3)='+00' THEN substr("+quote(k)+",1,length("+quote(k)+")-3)||'Z' ELSE "+quote(k)+" END)" if k in x['temporal_keys'] else quote(k)) for k in keys]
 rhs=[('julianday(?)' if k in x['temporal_keys'] else '?') for k in keys]
 for start in range(0,len(x['rows']),100):
  rows=x['rows'][start:start+100];args=[canonical_time(row[i]) if k in x['temporal_keys'] else row[i] for row in rows for k,i in zip(keys,positions)]
  left='('+','.join(lhs)+')' if len(keys)>1 else lhs[0]
  item='('+','.join(rhs)+')' if len(keys)>1 else rhs[0]
  if c.execute('SELECT 1 FROM '+quote(table)+' WHERE '+left+' IN ('+','.join(item for row in rows)+') LIMIT 1',args):raise ValueError('candidate key exists or timestamp-equivalent collision')
def check_unique_indexes(c,table,x):
 for idx in c.execute('PRAGMA index_list('+quote(table)+')'):
  if str(idx[2])!='1':continue
  if str(idx[4])!='0':raise ValueError('partial unique index requires specialized review')
  keys=[r[2] for r in c.execute('PRAGMA index_info('+quote(idx[1])+')')]
  if not keys or any(k is None or k not in x['columns'] for k in keys):raise ValueError('expression index requires specialized review')
  if keys==x['keys']:continue
  y=dict(x);y['keys']=keys;y['temporal_keys']=[k for k in keys if k in x['temporal_keys']]
  y['rows']=[r for r in x['rows'] if all(r[x['columns'].index(k)] is not None for k in keys)]
  seen=set()
  for row in y['rows']:
   key=tuple(canonical_time(row[x['columns'].index(k)]) if k in y['temporal_keys'] else row[x['columns'].index(k)] for k in keys)
   if key in seen:raise ValueError('candidate duplicate unique-index value')
   seen.add(key)
  check_absent(c,table,y)
def preflight(c,data):
 counts={}
 for table,x in data.items():
  if c.execute('SELECT type,name,tbl_name,sql FROM sqlite_master WHERE tbl_name=? ORDER BY type,name',[table])!=x['schema']:raise ValueError('schema changed')
  if [v[1] for v in c.execute('PRAGMA table_info('+quote(table)+')')]!=x['columns']:raise ValueError('column order changed')
  counts[table]=int(c.execute('SELECT count(*) FROM '+quote(table))[0][0])
  if counts[table]<x['initial_count']:raise ValueError('count decreased')
  check_absent(c,table,x)
  check_unique_indexes(c,table,x)
 return counts
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('candidate',type=Path);p.add_argument('--apply',action='store_true');p.add_argument('--expect-sha256');p.add_argument('--target',choices=['fleet-platform','fleet-platform-reconcile-stage-20261118'],default='fleet-platform');a=p.parse_args()
 digest=filehash(a.candidate)
 if a.apply and digest!=a.expect_sha256:p.error('reviewed candidate SHA required')
 data=json.loads(a.candidate.read_text());validate(data)
 cmd=['fleet','secrets','get','turso.fleet-platform-rw-token'] if a.target=='fleet-platform' else ['turso','db','tokens','create',a.target,'--expiration','1d']
 token=subprocess.run(cmd,capture_output=True,text=True,check=True,timeout=60).stdout.strip().splitlines()[-1];c=Client(token,a.target);begun=False;commit_attempted=False
 try:
  if a.apply:
   c.execute('PRAGMA foreign_keys=ON');assert c.execute('PRAGMA foreign_keys')==[['1']]
   c.execute('BEGIN IMMEDIATE');begun=True
  initial_counts=preflight(c,data);print(json.dumps({'mode':'apply' if a.apply else 'dry-run','candidate_sha256':digest,'tables':len(data),'initial_counts':initial_counts,'rows':sum(len(v['rows']) for v in data.values())}),flush=True)
  if not a.apply:return
  for table,x in data.items():
   cols=x['columns']
   for batch in batches(x['rows']):
    sql='INSERT INTO '+quote(table)+' ('+','.join(quote(v) for v in cols)+') VALUES '+','.join('('+','.join('?' for _ in cols)+')' for row in batch)
    c.execute(sql,[v for row in batch for v in row])
   assert int(c.execute('SELECT count(*) FROM '+quote(table))[0][0])==initial_counts[table]+len(x['rows'])
   print(table,len(x['rows']),'inserted',flush=True)
  if c.execute('PRAGMA foreign_key_check'):raise ValueError('foreign key violation')
  if filehash(a.candidate)!=digest:raise ValueError('candidate changed')
  commit_attempted=True;c.execute('COMMIT');begun=False;print('COMMIT acknowledged',flush=True)
 except Exception:
  if begun and not commit_attempted:
   try:c.execute('ROLLBACK');print('ROLLBACK acknowledged',flush=True)
   except Exception:print('ROLLBACK uncertain; inspect before retry',flush=True)
  elif commit_attempted:print('COMMIT uncertain; inspect before retry',flush=True)
  raise
 finally:
  try:c.close()
  except Exception:pass
if __name__=='__main__':main()
