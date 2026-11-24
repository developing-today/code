// External adapter validation with a Hrana-shaped fetch harness over native
// SQLite. It checks value encoding, atomic rollback, skipped writes, signed
// authentication, stale/tampered/unsigned rejection, replay prevention,
// forged identity stripping, and the loopback gateway read/write/search and
// denied cases including a gateway restart. It is not evidence of
// deployment, live Turso compatibility or migration completeness.
import {DatabaseSync} from 'node:sqlite';
import {readFileSync} from 'node:fs';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import {createTurso,atomicBatch} from './turso.mjs';
import worker from './worker.mjs';
import {makeGateway} from './gateway/proxy.mjs';

const KEY='external-test-hmac-key-0123456789abcdef';
const ORIGIN='https://gateway.example.ts.net';
const TURSO_URL='https://db.example.test';
const TOKEN='test-token';
const env={TURSO_URL,TURSO_AUTH_TOKEN:TOKEN,GATEWAY_HMAC_KEY:KEY,GATEWAY_ORIGIN:ORIGIN};

// --- Migration setup: same files and split rules as external/migrate.mjs.
const sql=new DatabaseSync(':memory:');
for(const rel of ['../drizzle/0000_tracker.sql','../drizzle/0001_store_defaults.sql','0002_gateway.sql','0003_steering_outbox.sql']){
  const text=readFileSync(new URL(rel,import.meta.url),'utf8');
  const parts=(rel==='0002_gateway.sql'||rel==='0003_steering_outbox.sql')?text.split(';'):text.split('--> statement-breakpoint');
  for(const part of parts)if(part.trim())sql.exec(part);
}
sql.exec('CREATE TABLE scratch (t TEXT, i INTEGER, f REAL, n TEXT)');

// --- Hrana v2 harness: execute/batch against the in-memory SQLite database.
function hranaValue(v){
  if(v===null)return {type:'null'};
  if(typeof v==='bigint')return {type:'integer',value:String(v)};
  if(typeof v==='number')return Number.isInteger(v)?{type:'integer',value:String(v)}:{type:'float',value:v};
  if(typeof v==='string')return {type:'text',value:v};
  throw Error('Unsupported harness value');
}
function argValue(a){
  if(a.type==='null')return null;
  if(a.type==='integer')return Number(a.value);
  if(a.type==='float')return a.value;
  if(a.type==='text')return a.value;
  throw Error('Unsupported harness arg');
}
function execSql(stmt){
  if(/^\s*(BEGIN|COMMIT|ROLLBACK)\b/i.test(stmt.sql)){sql.exec(stmt.sql);return {cols:[],rows:[],affected_row_count:0,last_insert_rowid:null}}
  const st=sql.prepare(stmt.sql),args=(stmt.args||[]).map(argValue),cols=st.columns();
  if(cols.length){st.setReturnArrays(true);return {cols:cols.map(c=>({name:c.name})),rows:st.all(...args).map(r=>r.map(hranaValue)),affected_row_count:0,last_insert_rowid:null}}
  const r=st.run(...args);
  return {cols:[],rows:[],affected_row_count:Number(r.changes),last_insert_rowid:r.lastInsertRowid!=null?String(r.lastInsertRowid):null};
}
function execBatch(steps){
  const results=[],errors=[];
  const ok=i=>results[i]!=null&&errors[i]==null;
  const cond=c=>{if(!c)return true;if(c.type==='ok')return ok(c.step);if(c.type==='not')return !cond(c.cond);if(c.type==='and')return c.conds.every(cond);if(c.type==='or')return c.conds.some(cond);throw Error('Unknown condition '+c.type)};
  for(const step of steps){
    if(!cond(step.condition)){results.push(null);errors.push(null);continue}
    try{results.push(execSql(step.stmt));errors.push(null)}catch(e){results.push(null);errors.push({message:String(e?.message||e),code:'SQLITE'})}
  }
  return {step_results:results,step_errors:errors};
}
async function harnessFetch(url,init){
  assert.equal(new URL(url).pathname,'/v2/pipeline');
  assert.equal(init.headers.Authorization,'Bearer '+TOKEN);
  const body=JSON.parse(init.body),results=[];
  for(const req of body.requests){
    if(req.type==='close'){results.push({type:'ok',response:{type:'close'}});continue}
    if(req.type==='execute'){try{results.push({type:'ok',response:{type:'execute',result:execSql(req.stmt)}})}catch(e){results.push({type:'error',error:{message:String(e?.message||e),code:'SQLITE'}})}continue}
    if(req.type==='batch'){results.push({type:'ok',response:{type:'batch',result:execBatch(req.batch.steps)}});continue}
    throw Error('Unexpected pipeline request '+req.type);
  }
  return {ok:true,status:200,json:async()=>({results})};
}
// The Worker builds its Turso adapter with the global fetch; route it to the
// harness. Real fetch is kept for the loopback gateway client calls below.
const realFetch=globalThis.fetch;
globalThis.fetch=async(url,init)=>{if(String(url).startsWith(TURSO_URL))return harnessFetch(url,init);throw Error('Unexpected fetch in external test: '+url)};

// --- Part A: adapter encoding, atomic rollback, skipped writes.
{
  const shape=atomicBatch([{sql:'SELECT 1',args:[],want_rows:true},{sql:'SELECT 2',args:[],want_rows:true}]);
  assert.equal(shape.steps.length,5);assert.equal(shape.commit,3);
  assert.equal(shape.steps[3].stmt.sql,'COMMIT');assert.equal(shape.steps[4].stmt.sql,'ROLLBACK');
  const db=createTurso({url:TURSO_URL,token:TOKEN,fetcher:harnessFetch});
  await db.prepare('INSERT INTO scratch VALUES (?,?,?,?)').bind('héllo 🌙',42,1.25,null).run();
  assert.deepEqual(await db.prepare('SELECT t,i,f,n FROM scratch WHERE i=?').bind(42).first(),{t:'héllo 🌙',i:42,f:1.25,n:null});
  await assert.rejects(db.batch([
    db.prepare('INSERT INTO scratch VALUES (?,?,?,?)').bind('rolled-back',7,0.5,null),
    db.prepare('INSERT INTO missing_table VALUES (1)')
  ]));
  assert.equal((await db.prepare("SELECT COUNT(*) AS n FROM scratch WHERE t='rolled-back'").first()).n,0);
  await assert.rejects(db.batch([
    db.prepare('INSERT INTO missing_table VALUES (1)'),
    db.prepare('INSERT INTO scratch VALUES (?,?,?,?)').bind('skipped',8,0.5,null)
  ]));
  assert.equal((await db.prepare("SELECT COUNT(*) AS n FROM scratch WHERE t='skipped'").first()).n,0);
  console.log('PASS external adapter: value encoding round-trip, atomic batch rollback, conditioned steps skip later writes after a failure.');
}

// --- Signed-request helpers mirroring external/gateway/proxy.mjs exactly.
function signedRequest({method='GET',path,actor={id:'ts_alice',email:'alice@example.com'},body,nonce=crypto.randomUUID(),time=Math.floor(Date.now()/1000),key=KEY,origin=ORIGIN,extraHeaders={}}){
  const raw=body===undefined?'':JSON.stringify(body);
  const actorB64=Buffer.from(JSON.stringify(actor)).toString('base64url');
  const hash=crypto.createHash('sha256').update(raw).digest('hex');
  const canonical=['bedaway-gateway-v1',String(time),nonce,method,path,actorB64,origin,hash].join('\n');
  const sig=crypto.createHmac('sha256',key).update(canonical).digest('hex');
  const headers={'x-bedaway-time':String(time),'x-bedaway-nonce':nonce,'x-bedaway-actor':actorB64,'x-bedaway-origin':origin,'x-bedaway-signature':sig,...extraHeaders};
  if(method==='POST')headers['content-type']='application/json';
  return new Request('https://worker.example.test'+path,{method,headers,...(method==='POST'?{body:raw}:{})});
}
const callWorker=async req=>{const r=await worker.fetch(req,env);return [r.status,await r.json().catch(()=>({}))]};
const state={items:[],tags:[],people:['Shared'],notes:[],inventory:[],prefs:{},rules:{},recs:{},overrides:{}};

// --- Part B: direct Worker calls fail closed; signed calls work exactly once.
let sid;
{
  assert.equal((await callWorker(new Request('https://worker.example.test/api/bootstrap')))[0],401);
  assert.equal((await callWorker(signedRequest({path:'/api/bootstrap',key:'wrong-hmac-key-wrong-hmac-key-0000'})))[0],401);
  assert.equal((await callWorker(signedRequest({path:'/api/bootstrap',time:Math.floor(Date.now()/1000)-3600})))[0],401);
  assert.equal((await callWorker(signedRequest({path:'/api/bootstrap',origin:'https://evil.example.ts.net'})))[0],401);
  // Tampered body: headers sign one payload, different bytes are sent.
  const signed=signedRequest({method:'POST',path:'/api/stores',body:{name:'Signed',type:'private',initialState:state}});
  const tampered=new Request(signed.url,{method:'POST',headers:signed.headers,body:JSON.stringify({name:'Tampered',type:'private',initialState:state})});
  assert.equal((await callWorker(tampered))[0],401);

  let [code,r]=await callWorker(signedRequest({path:'/api/bootstrap'}));
  assert.equal(code,200);assert.deepEqual(r.stores,[]);
  [code,r]=await callWorker(signedRequest({method:'POST',path:'/api/stores',body:{name:'External',type:'private',initialState:state}}));
  assert.equal(code,201);sid=r.store.id;
  const withItem={...state,items:[{id:'one',name:'Lamp',tags:[],attributes:[]}]};
  [code,r]=await callWorker(signedRequest({method:'POST',path:'/api/stores/'+sid+'/events',body:{expectedVersion:0,event:{id:'ev_1',label:'Add lamp',after:withItem}}}));
  assert.equal(code,200);assert.equal(r.version,1);
  [code,r]=await callWorker(signedRequest({path:'/api/stores/'+sid}));
  assert.equal(code,200);assert.equal(r.state.items[0].name,'Lamp');
  [code,r]=await callWorker(signedRequest({path:'/api/search?q=Lamp'}));
  assert.equal(code,200);assert(r.results.some(x=>x.storeId===sid));

  // Replay: identical signed headers a second time are rejected (nonce spent).
  const replay=signedRequest({path:'/api/bootstrap'});
  assert.equal((await callWorker(replay))[0],200);
  const replay2=new Request(replay.url,{method:'GET',headers:replay.headers});
  assert.equal((await callWorker(replay2))[0],401);

  // Forged browser identity headers are ignored; the signed actor decides.
  [code,r]=await callWorker(signedRequest({path:'/api/bootstrap',extraHeaders:{'oai-authenticated-user-id':'mallory','oai-authenticated-user-email':'mallory@example.com'}}));
  assert.equal(code,200);assert(r.stores.some(s=>s.id===sid));assert(!JSON.stringify(r).includes('mallory'));
  // Another signed principal cannot see the private store.
  const bob={id:'ts_bob',email:'bob@example.com'};
  assert.equal((await callWorker(signedRequest({path:'/api/stores/'+sid,actor:bob})))[0],404);
  [code,r]=await callWorker(signedRequest({path:'/api/bootstrap',actor:bob}));
  assert.equal(code,200);assert(!r.stores.some(s=>s.id===sid));
  console.log('PASS external worker: unsigned/wrong-key/stale/wrong-origin/tampered calls rejected with 401, signed read/write/search succeed, nonce replay rejected, forged identity headers stripped, private store invisible to other principals.');
}

// --- Part C: loopback gateway read/write/search, denied cases, restart.
async function listen(server){await new Promise(r=>server.listen(0,'127.0.0.1',r));return server.address().port}
async function close(server){await new Promise(r=>server.close(r))}
const gatewayOptions=()=>({workerOrigin:'https://worker.example.test',origin:ORIGIN,key:KEY,allowedLogins:new Set(['alice@example.com']),fetcher:async(url,init)=>worker.fetch(new Request(url,{method:init.method,headers:init.headers,body:init.body}),env)});
{
  const gw1=makeGateway(gatewayOptions()),port1=await listen(gw1),base1='http://127.0.0.1:'+port1;
  const login={'tailscale-user-login':'alice@example.com'};
  let res=await realFetch(base1+'/api/bootstrap',{headers:login});
  // The gateway derives its own principal (ts_<sha256 of the login>), distinct
  // from Part B's directly signed actor, so Part B's private store is absent.
  assert.equal(res.status,200);assert(!(await res.json()).stores.some(s=>s.id===sid));
  res=await realFetch(base1+'/api/stores',{method:'POST',headers:{...login,'content-type':'application/json',origin:ORIGIN},body:JSON.stringify({name:'ViaGateway',type:'private',initialState:{...state,items:[{id:'gw1',name:'Gateway Lamp',tags:[],attributes:[]}]}})});
  assert.equal(res.status,201);const gwSid=(await res.json()).store.id;
  res=await realFetch(base1+'/api/search?q=Gateway',{headers:login});
  assert.equal(res.status,200);assert((await res.json()).results.some(x=>x.storeId===gwSid));
  // Denied cases: no login, unlisted login, cross-origin write.
  assert.equal((await realFetch(base1+'/api/bootstrap')).status,403);
  assert.equal((await realFetch(base1+'/api/bootstrap',{headers:{'tailscale-user-login':'mallory@example.com'}})).status,403);
  assert.equal((await realFetch(base1+'/api/stores',{method:'POST',headers:{...login,'content-type':'application/json',origin:'https://evil.test'},body:'{}'})).status,403);
  await close(gw1);
  // Restart: a fresh gateway process over the same database serves prior data
  // and accepts new writes; state lives in the database, not the gateway.
  const gw2=makeGateway(gatewayOptions()),port2=await listen(gw2),base2='http://127.0.0.1:'+port2;
  res=await realFetch(base2+'/api/bootstrap',{headers:login});
  assert.equal(res.status,200);assert((await res.json()).stores.some(s=>s.id===gwSid));
  res=await realFetch(base2+'/api/stores/'+gwSid+'/events',{method:'POST',headers:{...login,'content-type':'application/json',origin:ORIGIN},body:JSON.stringify({expectedVersion:0,event:{id:'gw_ev',label:'After restart',after:{...state,items:[{id:'gw1',name:'Gateway Lamp',tags:[],attributes:[]},{id:'gw2',name:'Second Lamp',tags:[],attributes:[]}]}}})});
  assert.equal(res.status,200);assert.equal((await res.json()).version,1);
  await close(gw2);
  console.log('PASS external gateway: allowed login reads/writes/searches through the signed path, missing/unlisted login and cross-origin writes denied with 403, restart preserves data and write access.');
}
globalThis.fetch=realFetch;
console.log('PASS external: adapter, signed Worker boundary and gateway loop verified against a Hrana-shaped fixture (not a live deployment).');
