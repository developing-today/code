"""Bounded SELECT-only Hrana pipeline; preserves existing Client transaction/host rules."""
import json,urllib.request,urllib.error,math,re
from urllib.parse import urlsplit
from import_runtime import OPEN,arg

def execute_reads(c,queries):
 results=[]
 for start in range(0,len(queries),20):
  requests=[]
  for sql,values in queries[start:start+20]:
   if not sql.lstrip().upper().startswith('SELECT ') or ';' in sql:raise ValueError('guard pipeline accepts single SELECT only')
   requests.append({'type':'execute','stmt':{'sql':sql,'args':[arg(v) for v in values],'want_rows':True}})
  body={'requests':requests}
  if c.baton:body['baton']=c.baton
  payload=json.dumps(body).encode()
  if len(payload)>2000000:raise ValueError('read pipeline exceeds safe payload limit')
  req=urllib.request.Request(c.url,data=payload,headers={'Authorization':'Bearer '+c.token,'Content-Type':'application/json'})
  try:
   with OPEN.open(req,timeout=90) as response:out=json.load(response)
  except urllib.error.HTTPError as e:raise RuntimeError('read pipeline HTTP '+str(e.code)) from None
  if not isinstance(out,dict) or not isinstance(out.get('results'),list) or len(out['results'])!=len(requests):raise RuntimeError('invalid pipeline result count')
  c.baton=out.get('baton');base=out.get('base_url')
  if base:
   u=urlsplit(base)
   if u.scheme!='https' or u.hostname!=c.host or u.username or u.password or u.port not in (None,443) or u.query or u.fragment or u.path not in ('','/'):raise RuntimeError('unexpected service redirect')
   c.url=base.rstrip('/')+'/v2/pipeline'
  if c.transaction and (not isinstance(c.baton,str) or not c.baton):raise RuntimeError('transaction baton lost')
  for result in out['results']:
   if not isinstance(result,dict) or result.get('type')!='ok':raise RuntimeError('guard pipeline SQL failure')
   response=result.get('response',{})
   if not isinstance(response,dict) or response.get('type')!='execute' or not isinstance(response.get('result'),dict) or not isinstance(response['result'].get('rows'),list):raise RuntimeError('invalid pipeline execute response')
   cols=response['result'].get('cols')
   if not isinstance(cols,list) or not cols or any(not isinstance(col,dict) or not isinstance(col.get('name'),str) for col in cols):raise RuntimeError('invalid pipeline column metadata')
   rows=response['result']['rows']
   if any(not isinstance(row,list) or len(row)!=len(cols) for row in rows):raise RuntimeError('pipeline row width mismatch')
   if any(not isinstance(row,list) or any(not isinstance(cell,dict) or cell.get('type') not in ('null','integer','float','text','blob') for cell in row) for row in rows):raise RuntimeError('invalid pipeline row shape')
   for row in rows:
    for cell in row:
     typ=cell['type'];v=cell.get('value')
     if typ=='null':
      if v is not None:raise RuntimeError('invalid null cell')
     elif 'value' not in cell:raise RuntimeError('missing pipeline cell value')
     elif typ in ('text','blob') and not isinstance(v,str):raise RuntimeError('invalid text/blob cell')
     elif typ=='integer' and (isinstance(v,bool) or not isinstance(v,(str,int)) or re.fullmatch(r'-?[0-9]+',str(v)) is None):raise RuntimeError('invalid integer cell')
     elif typ=='float':
      if isinstance(v,bool) or not isinstance(v,(str,int,float)):raise RuntimeError('invalid float cell')
      try:valid=math.isfinite(float(v))
      except (TypeError,ValueError):valid=False
      if not valid:raise RuntimeError('nonfinite/invalid float cell')
   results.append([[cell.get('value') for cell in row] for row in rows])
 return results
