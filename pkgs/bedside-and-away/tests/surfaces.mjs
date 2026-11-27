// BA-W01 L7 (contract §5, §8 test 1): history, media, exports, summaries and background jobs go through
// the §4 evaluator with the initiating principal, and each answers a forbidden entity exactly like an
// absent one.
import {DatabaseSync} from 'node:sqlite';
import {readFileSync} from 'node:fs';
import assert from 'node:assert/strict';
import {webcrypto} from 'node:crypto';
globalThis.crypto??=webcrypto;
import {NOT_FOUND,EXPORT_SCHEMA,historyView,mediaLookup,buildExport,summaryInput,jobEnvelope,authorizeJobTargets,jobAuthorizer} from '../worker/surfaces.mjs';
import {createSteeringOutbox} from '../worker/steering-outbox.mjs';
import {api} from '../worker/api.mjs';

const access={schemaVersion:1,groups:{},policies:{'item:b':{rules:[{effect:'deny',subject:{type:'user',id:'bob'},actions:['read','write','admin']}]}}};
const state={items:[{id:'a',name:'Lamp',tags:[],attributes:[]},{id:'b',name:'Hidden gift',tags:[],attributes:[]}],tags:[],people:[],notes:[{id:'n1',itemId:'a',text:'ok'},{id:'n2',itemId:'b',text:'secret'}],inventory:[{id:'i1',itemId:'a',qty:1,mode:'Bought'},{id:'i2',itemId:'b',qty:5,mode:'Bought'}],prefs:{},rules:{},overrides:{},recs:{},access};
const bob={principal:{id:'bob'},role:'editor'},alice={principal:{id:'alice'},role:'admin'};

// Exports: only readable rows, with the source revision stated; policy never leaks to a non-admin.
{
 const out=buildExport(state,bob,{storeId:'s',sourceRevision:7});
 assert.equal(out.schema,EXPORT_SCHEMA);assert.equal(out.sourceRevision,7);assert.equal(out.storeId,'s');
 assert.deepEqual(out.state.items.map(x=>x.id),['a']);
 assert.deepEqual(out.state.notes.map(x=>x.id),['n1']);
 assert.equal(out.state.access,undefined);
 assert.deepEqual(buildExport(state,alice,{storeId:'s',sourceRevision:7}).state.items.map(x=>x.id),['a','b']);
 assert.throws(()=>buildExport(state,bob,{storeId:'s',sourceRevision:-1}),/revision/);
}

// Media: a forbidden entity and an absent one give the same answer.
{
 assert.equal(mediaLookup(state,bob,{kind:'item',id:'a'}).name,'Lamp');
 assert.deepEqual(mediaLookup(state,bob,{kind:'item',id:'b'}),mediaLookup(state,bob,{kind:'item',id:'missing'}));
 assert.equal(mediaLookup(state,bob,{kind:'item',id:'b'}),null);
 assert.equal(mediaLookup(state,bob,{kind:'note',id:'n2'}),null);
 assert.equal(mediaLookup(state,bob,{kind:'policy',id:'a'}),null);
 assert.equal(mediaLookup(state,alice,{kind:'item',id:'b'}).name,'Hidden gift');
}

// Summaries: aggregates are computed over readable rows only, so totals never count a hidden row.
{
 const qty=ctx=>summaryInput(state,ctx).inventory.reduce((a,r)=>a+r.qty,0);
 assert.equal(qty(bob),1);assert.equal(qty(alice),6);
}

// History: a scoped store's history is withheld rather than redacted to placeholders.
{
 const events=[{id:'e1',label:'Added lamp',before:{},after:{}}];
 assert.deepEqual(historyView(events,state,bob),[]);
 const open={...state,access:undefined};
 assert.deepEqual(historyView(events,open,bob).map(e=>e.id),['e1']);
}

// Background jobs: authorized as the initiator only; a denied target looks like an absent one.
{
 assert.throws(()=>jobEnvelope({principal:null,role:'editor',storeId:'s'}),/principal/);
 const asBob=jobEnvelope({principal:{id:'bob'},role:'editor',storeId:'s'});
 assert.ok(Object.isFrozen(asBob)&&Object.isFrozen(asBob.initiator));
 assert.deepEqual(authorizeJobTargets(state,asBob,[{kind:'item',id:'a'}]),{allowed:true});
 const denied=authorizeJobTargets(state,asBob,[{kind:'item',id:'b'}]),absent=authorizeJobTargets(state,asBob,[{kind:'item',id:'missing'}]);
 assert.deepEqual(denied,absent);assert.deepEqual(denied,{...NOT_FOUND});
 assert.deepEqual(authorizeJobTargets(state,asBob,[{kind:'item',id:'a'},{kind:'item',id:'b'}]),{...NOT_FOUND});
 assert.deepEqual(authorizeJobTargets(state,asBob,[]),{...NOT_FOUND});
 assert.deepEqual(authorizeJobTargets(state,asBob,[{kind:'report',id:'spend'}]),{allowed:true});
 const asReader=jobEnvelope({principal:{id:'bob'},role:'reader',storeId:'s'});
 assert.deepEqual(authorizeJobTargets(state,asReader,[{kind:'item',id:'a'}]),{...NOT_FOUND});
 const asAlice=jobEnvelope({principal:{id:'alice'},role:'admin',storeId:'s'});
 assert.deepEqual(authorizeJobTargets(state,asAlice,[{kind:'item',id:'b'}]),{allowed:true});
}

// The steering outbox, wired to the evaluator: Bob cannot enqueue a job over an item he can't see.
{
 const sql=new DatabaseSync(':memory:');
 sql.exec("CREATE TABLE stores(id TEXT PRIMARY KEY,version INTEGER,archived INTEGER);CREATE TABLE memberships(store_id TEXT,user_id TEXT,role TEXT);INSERT INTO stores VALUES('s',3,0);INSERT INTO memberships VALUES('s','bob','editor');");
 sql.exec(readFileSync(new URL('../external/0003_steering_outbox.sql',import.meta.url),'utf8'));
 const db={prepare(q){let b=[];return {bind(...v){b=v;return this},async first(){return sql.prepare(q).get(...b)||null},async run(){return {meta:{changes:Number(sql.prepare(q).run(...b).changes)}}}}},async batch(xs){sql.exec('BEGIN');try{const out=[];for(const x of xs)out.push(await x.run());sql.exec('COMMIT');return out}catch(e){sql.exec('ROLLBACK');throw e}}};
 const request=id=>({id:'r-'+id,storeId:'s',schemaVersion:2,revision:1,status:'pending',targets:[{kind:'item',id,before:{id}}],request:'Improve it'});
 let pending;
 const authorize=jobAuthorizer(async({actorId})=>actorId==='bob'?{state,role:'editor',version:3,request:pending,targetsFresh:true}:null);
 assert.equal((await authorize({storeId:'s',requestId:'x',actorId:'mallory'})).allowed,false);
 const config={enabled:true,operatorApproved:true,endpoint:'https://agent.test/hooks',allowedDestinations:['https://agent.test/hooks'],recipient:'Archer',secret:'s'.repeat(32)};
 const outbox=createSteeringOutbox({db,config,authorize,clock:()=>1000});
 pending=request('a');
 assert.equal((await outbox.enqueue({storeId:'s',requestId:'r-a',revision:1,actorId:'bob'})).state,'pending');
 pending=request('b');
 await assert.rejects(()=>outbox.enqueue({storeId:'s',requestId:'r-b',revision:1,actorId:'bob'}),/revoked/);
 assert.equal(await outbox.get('s:r-b:1'),null);
}

// The export route answers as its builder does, through the same store access as every other route.
{
 const sql=new DatabaseSync(':memory:');
 for(const f of ['0000_tracker.sql','0001_store_defaults.sql'])sql.exec(readFileSync(new URL('../drizzle/'+f,import.meta.url),'utf8'));
 const db={prepare(q){let binds=[];return {bind(...v){binds=v;return this},async first(){return sql.prepare(q).get(...binds)||null},async all(){return {results:sql.prepare(q).all(...binds)}},async run(){const r=sql.prepare(q).run(...binds);return {meta:{changes:Number(r.changes)}}}}},async batch(statements){sql.exec('BEGIN');try{const results=[];for(const s of statements)results.push(await s.run());sql.exec('COMMIT');return results}catch(e){sql.exec('ROLLBACK');throw e}}};
 const call=async(path,{user='carol',data}={})=>{const r=await api(new Request('https://app.test/api/'+path,{method:data?'POST':'GET',headers:{'oai-authenticated-user-id':user,'oai-authenticated-user-email':user+'@example.com',...(data?{origin:'https://app.test','content-type':'application/json'}:{})},...(data?{body:JSON.stringify(data)}:{})}),{DB:db});return [r.status,await r.json()]};
 const [code,r]=await call('stores',{data:{name:'Carol shared',type:'shared',initialState:{...state,access:undefined}}});
 assert.equal(code,201);
 const [ok,out]=await call('stores/'+r.store.id+'/export');
 assert.equal(ok,200);assert.equal(out.schema,EXPORT_SCHEMA);assert.equal(out.storeId,r.store.id);assert.equal(out.sourceRevision,0);
 assert.deepEqual(out.state.items.map(x=>x.id),['a','b']);
 assert.deepEqual(await call('stores/'+r.store.id+'/export',{user:'bob'}),await call('stores/'+crypto.randomUUID()+'/export',{user:'bob'}));
}

console.log('PASS L7 surfaces: exports, media, summaries, history and jobs decide through the evaluator as the initiator; forbidden answers like absent.');
