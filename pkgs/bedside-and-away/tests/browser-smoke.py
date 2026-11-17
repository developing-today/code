#!/usr/bin/env python3
"""Disposable SQLite/loopback UI smoke. Requires installed node/chromedriver/chromium; no installs."""
import json,os,sys,time,subprocess,socket,urllib.request,urllib.error,shutil,tempfile
root=os.path.abspath(sys.argv[1] if len(sys.argv)>1 else os.getcwd())
E='element-6066-11e4-a52e-4f735466cecf';results=[];session=None;driver=None;server=None
log=tempfile.TemporaryFile(mode='w+')
def req(method,path,data=None):
 r=urllib.request.Request(base+path,data=json.dumps(data).encode() if data is not None else None,method=method,headers={'Content-Type':'application/json'})
 try:
  with urllib.request.urlopen(r,timeout=30) as response:out=json.load(response)
 except urllib.error.HTTPError as e:raise RuntimeError(e.read().decode())
 if isinstance(out.get('value'),dict) and out['value'].get('error'):raise RuntimeError(out['value'])
 return out.get('value')
def call(method,path,data=None):return req(method,'/session/'+session+path,data)
def find(selector,timeout=12):
 deadline=time.time()+timeout;error=None
 while time.time()<deadline:
  try:return call('POST','/element',{'using':'css selector','value':selector})[E]
  except Exception as e:error=e;time.sleep(.15)
 raise error
def click(selector):call('POST','/element/'+find(selector)+'/click',{});time.sleep(.25)
def fill(selector,value):
 el=find(selector);call('POST','/element/'+el+'/clear',{});call('POST','/element/'+el+'/value',{'text':value});
def text(selector):return call('GET','/element/'+find(selector)+'/text')
def js(script,*args):return call('POST','/execute/sync',{'script':script,'args':list(args)})
def submit():click('#dialogform button[type=submit]');time.sleep(.35)
def nav(view):click('nav [data-view="'+view+'"]')
def mark(name):results.append(name);print('PASS '+name,flush=True)
try:
 server=subprocess.Popen(['node','tests/local-server.mjs'],cwd=root,stdout=subprocess.PIPE,stderr=log,text=True)
 url=server.stdout.readline().strip();assert url.startswith('http://127.0.0.1:'),url
 with socket.socket() as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
 base='http://127.0.0.1:'+str(port)
 driver=subprocess.Popen(['chromedriver','--port='+str(port),'--allowed-ips=127.0.0.1'],stdout=log,stderr=log)
 for _ in range(60):
  try:req('GET','/status');break
  except Exception:time.sleep(.1)
 chrome=shutil.which('chromium') or shutil.which('chromium-browser') or shutil.which('google-chrome')
 caps={'browserName':'chrome','goog:chromeOptions':{'binary':chrome,'args':['--headless=new','--disable-background-networking','--no-first-run','--disable-default-apps','--disable-sync','--window-size=1280,900']},'goog:loggingPrefs':{'browser':'ALL'}}
 session=req('POST','/session',{'capabilities':{'alwaysMatch':caps}})['sessionId']
 call('POST','/url',{'url':url});click('[data-act="store-provision"]');find('[data-card]');mark('provision and tracker')
 first=find('[data-card]:nth-child(2)');click('[data-card]:nth-child(2) [data-act="edit"]');fill('#dialogform [name="name"]','Fixture edited option');submit();assert 'Fixture edited option' in text('#app');mark('edit persisted')
 click('[data-card]:nth-child(2) [data-act="edit"]');fill('#dialogform [name="name"]','Cancelled fixture title');click('#dialogform [data-act="close"]');assert 'Cancelled fixture title' not in text('#app');mark('cancel edit preserves state')
 nav('intake');click('[data-act="report-receipt"]');fill('#dialogform [name="receipt"]','Fixture edited option | 9.99 | 2');fill('#dialogform [name="store"]','Fixture shop');submit();assert text('#dialogform h2')=='Review receipt lines';click('#dialogform [data-act="close"]');nav('report');assert '$9.99' not in text('#app');mark('receipt cancel makes no write')
 click('[data-act="report-receipt"]');fill('#dialogform [name="receipt"]','Fixture edited option | 9.99 | 2');fill('#dialogform [name="store"]','Fixture shop');submit();submit();assert '$9.99' in text('#app');mark('reviewed receipt save')
 nav('decision');click('[data-act="report-group-decision"]');click('#dialogform [data-act="close"]');mark('decision dialog cancel')
 click('header [data-view="settings"]');click('[data-act="refresh"]');find('[data-act="rec-accept"]');click('[data-act="rec-cancel"]');assert 'cancelled' in text('#app');mark('defaults preview cancellation')
 nav('items');click('[data-card]:nth-child(2) input[data-pick]');click('[data-card]:nth-child(3) input[data-pick]');click('#app > .row [data-act="variants"]');fill('#dialogform [name="variantName"]','Fixture family');submit();assert 'Fixture family' in text('#app');mark('variant grouping')
 nav('steering');click('#app [data-act="steering-general"]');fill('#dialogform [name="request"]','Fixture general request');submit();assert 'Fixture general request' in text('#app');mark('general request queue')
 nav('items');click('[data-card]:nth-child(2) [data-act="steering-new"]');fill('#dialogform [name="request"]','Fixture priority request');submit();nav('steering');click('[data-act="steering-review"]');click('#dialogform [name="set_priority"]');fill('#dialogform [name="priority"]','90');submit();click('[data-act="steering-review"]');submit();assert 'applied' in text('#app').lower();mark('steering proposal and apply')
 for view in ['history','search','research','guide','items','report','intake','decision']:
  nav(view);assert text('#app').strip()
 mark('navigation views')
 call('POST','/window/rect',{'width':390,'height':844});nav('report');overflow=js('return document.documentElement.scrollWidth > window.innerWidth+2');assert not overflow, 'Mobile overflow on report';mark('mobile report no horizontal overflow')
 nav('items');overflow=js('return document.documentElement.scrollWidth > window.innerWidth+2');assert not overflow, 'Mobile overflow on tracker';mark('mobile tracker no horizontal overflow')
 logs=call('POST','/log',{'type':'browser'});severe=[x for x in logs if x.get('level')=='SEVERE' and 'favicon.ico' not in x.get('message','')];assert not severe,severe
 print(json.dumps({'passed':results,'browserErrors':severe},indent=2))
except Exception:
 print(json.dumps({'passedBeforeFailure':results},indent=2),flush=True)
 if session:
  try:print('VISIBLE_APP',text('#app')[:3000],flush=True);print('BROWSER_LOG',call('POST','/log',{'type':'browser'}),flush=True)
  except Exception:pass
 raise
finally:
 if session:
  try:call('DELETE','')
  except Exception:pass
 for proc in [driver,server]:
  if proc:proc.terminate()
