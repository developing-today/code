#!/usr/bin/env python3
"""Insert only reviewed missing AA source/sheet IDs. Default read-only dry run.
Usage: import_aa_missing.py CANDIDATE_JSON [--apply --expect-sha256 HASH]
Never updates source41 or any existing row; all79conflicts remain unchanged.
"""
import argparse,json,subprocess
from pathlib import Path
from import_runtime import Client,filehash,batches,quote
EXPECTED={'ainventory_sources':3,'ainventory_sheet_rows':157}
def validate(data):
 if set(data)!=set(EXPECTED):raise ValueError('unexpected table set')
 for t,n in EXPECTED.items():
  d=data[t];cols=d['columns'];rows=d['rows'];ids=d['allowed_ids']
  if not cols or cols[0]!='id' or len(cols)!=len(set(cols)):raise ValueError('invalid columns')
  if len(rows)!=n or len(set(ids))!=n or sorted(r[0] for r in rows)!=sorted(ids):raise ValueError('invalid allowed ID set')
  if any(len(r)!=len(cols) for r in rows):raise ValueError('row width mismatch')
  if not isinstance(d['expected_existing_count'],int):raise ValueError('invalid baseline count')
def preflight(c,data):
 for t,d in data.items():
  actual_cols=[r[1] for r in c.execute('PRAGMA table_info('+quote(t)+')')]
  if actual_cols!=d['columns']:raise ValueError('target column map changed')
  count=int(c.execute('SELECT count(*) FROM '+quote(t))[0][0])
  if count!=d['expected_existing_count']:raise ValueError('baseline count changed')
  if c.execute('SELECT id FROM '+quote(t)+' WHERE id IN ('+','.join('?' for _ in d['allowed_ids'])+')',d['allowed_ids']):raise ValueError('allowed ID already exists')
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('candidate',type=Path);p.add_argument('--apply',action='store_true');p.add_argument('--expect-sha256');p.add_argument('--target',choices=['fleet-platform','fleet-supabase-runtime-legacy-stage-20261118'],default='fleet-platform');a=p.parse_args();digest=filehash(a.candidate)
 if a.apply and digest!=a.expect_sha256:p.error('matching reviewed candidate hash required')
 data=json.loads(a.candidate.read_text());validate(data)
 token=subprocess.run(['fleet','secrets','get','turso.fleet-platform-rw-token'],capture_output=True,text=True,check=True,timeout=60).stdout.strip().splitlines()[-1];
 if a.target!='fleet-platform':token=subprocess.run(['turso','db','tokens','create',a.target,'--expiration','1d'],capture_output=True,text=True,check=True,timeout=60).stdout.strip()
 c=Client(token,a.target);begun=False;attempt=False;committed=False
 try:
  if a.apply:
   c.execute('PRAGMA foreign_keys=ON')
   if c.execute('PRAGMA foreign_keys')!=[['1']]:raise RuntimeError('FK not enabled')
   c.execute('BEGIN IMMEDIATE');begun=True
  preflight(c,data);print(json.dumps({'mode':'apply' if a.apply else 'dry-run','candidate_sha256':digest,'insert_counts':EXPECTED,'existing_conflicts_preserved':79}),flush=True)
  if not a.apply:return
  for t,d in data.items():
   for batch in batches(d['rows']):
    c.execute('INSERT INTO '+quote(t)+' ('+','.join(quote(x) for x in d['columns'])+') VALUES '+','.join('('+','.join('?' for _ in d['columns'])+')' for _ in batch),[v for r in batch for v in r])
   if int(c.execute('SELECT count(*) FROM '+quote(t))[0][0])!=d['expected_existing_count']+len(d['rows']):raise RuntimeError('postinsert count mismatch')
  if filehash(a.candidate)!=digest:raise RuntimeError('candidate changed')
  if c.execute('PRAGMA foreign_key_check'):raise RuntimeError('FK verification failed')
  attempt=True;c.execute('COMMIT');committed=True;print('Committed160 missing rows; no existing values updated',flush=True)
 except BaseException:
  if attempt and not committed:print('COMMIT outcome uncertain; do not retry without reconciliation',flush=True)
  elif begun:
   try:c.execute('ROLLBACK');print('Rollback acknowledged',flush=True)
   except Exception:print('Rollback unverified; inspect destination',flush=True)
  raise
 finally:
  try:c.close()
  except Exception:pass
if __name__=='__main__':main()
