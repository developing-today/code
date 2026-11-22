// BA-W01 §8 test 1 (API surfaces): a nonmember cannot tell a store that exists from one that doesn't.
// Every store route must answer a forbidden store exactly as it answers an absent one, and search must
// neither return nor count rows the caller cannot read.
import {DatabaseSync} from 'node:sqlite';
import {readFileSync} from 'node:fs';
import assert from 'node:assert/strict';
import {api} from '../worker/api.mjs';

const sql=new DatabaseSync(':memory:');
for(const f of ['0000_tracker.sql','0001_store_defaults.sql'])sql.exec(readFileSync(new URL('../drizzle/'+f,import.meta.url),'utf8'));
const db={prepare(q){let binds=[];return {bind(...v){binds=v;return this},async first(){return sql.prepare(q).get(...binds)||null},async all(){return {results:sql.prepare(q).all(...binds)}},async run(){const r=sql.prepare(q).run(...binds);return {meta:{changes:Number(r.changes)}}}}},async batch(statements){sql.exec('BEGIN');try{const results=[];for(const s of statements)results.push(await s.run());sql.exec('COMMIT');return results}catch(e){sql.exec('ROLLBACK');throw e}}};
const call=async(path,{user='alice',data}={})=>{const r=await api(new Request('https://app.test/api/'+path,{method:data?'POST':'GET',headers:{'oai-authenticated-user-id':user,'oai-authenticated-user-email':user+'@example.com',...(data?{origin:'https://app.test','content-type':'application/json'}:{})},...(data?{body:JSON.stringify(data)}:{})}),{DB:db});return [r.status,await r.json()]};

const base={items:[],tags:[],people:[],notes:[],inventory:[],prefs:{},rules:{},recs:{},overrides:{}};
const lamp={id:'lamp',name:'Brass lamp',tags:[],attributes:[]};
const stores={};
for(const type of ['private','shared']){
 const [code,r]=await call('stores',{user:'carol',data:{name:'Carol '+type,type,initialState:{...base,items:[lamp]}}});
 assert.equal(code,201);stores[type]=r.store.id;
}
// Archived open store: invisible to nonmembers.
{
 const [,r]=await call('stores',{user:'carol',data:{name:'Old open',type:'open',initialState:base}});
 stores.archivedOpen=r.store.id;
 const [code]=await call('stores/'+r.store.id+'/settings',{user:'carol',data:{name:'Old open',type:'open',archived:true}});
 assert.equal(code,200);
}
// A non-owner member of a private store is still a nonmember of it.
const absent=crypto.randomUUID();
const routes=[
 ['GET',''],['GET','/policy'],['GET','/access'],['GET','/bogus'],
 ...['events','steering','variants','policy','history'].map(s=>['POST','/'+s,{commandId:crypto.randomUUID(),expectedVersion:0}]),
 ['POST','/settings',{name:'x',type:'shared'}],['POST','/members/carol',{role:'viewer'}],['POST','/members/dave',{role:'remove'}],
 ['POST','/invites',{email:'bob@example.com',role:'viewer'}],['POST','/invites/'+crypto.randomUUID()+'/revoke',{}],
];
let compared=0;
for(const [method,suffix,data] of routes){
 const ask=id=>call('stores/'+id+suffix,{user:'bob',...(method==='POST'?{data:data??{}}:{})});
 const want=await ask(absent);
 assert.ok(want[0]>=400,'absent store must fail on '+method+' '+suffix);
 for(const [kind,id] of Object.entries(stores)){
  assert.deepEqual(await ask(id),want,`${method} ${suffix||'/'} on a ${kind} store answers differently from an absent one`);compared++;
 }
}
// Nothing the probes sent changed Carol's stores.
for(const id of [stores.private,stores.shared]){const [code,r]=await call('stores/'+id,{user:'carol'});assert.equal(code,200);assert.equal(r.version,0);assert.deepEqual(r.state.items.map(x=>x.id),['lamp'])}

// Search: no hits and no count from stores the caller can't read.
const [code,r]=await call('search?q=lamp',{user:'bob'});
assert.equal(code,200);assert.deepEqual(r,{query:'lamp',results:[],truncated:false});
assert.equal((await call('search?q=lamp',{user:'carol'}))[1].results.length,2);

console.log(`PASS existence oracle: ${compared} forbidden-store probes across ${routes.length} store routes answer exactly like an absent store; search returns and counts nothing unreadable.`);
