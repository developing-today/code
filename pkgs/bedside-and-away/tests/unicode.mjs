import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {DatabaseSync} from 'node:sqlite';
import {name,isComment,isIdentifier,visible} from '../worker/text.mjs';
import {api} from '../worker/api.mjs';
const corpus=JSON.parse(readFileSync(new URL('./unicode-corpus.json',import.meta.url),'utf8'));
for(const [text,ok] of corpus.comment)assert.equal(isComment(text),ok,'comment '+JSON.stringify(text));
for(const [text,ok] of corpus.identifier)assert.equal(isIdentifier(text),ok,'identifier '+JSON.stringify(text));
for(const [input,cleaned] of corpus.name)assert.equal(name(input),cleaned,'name '+JSON.stringify(input));
// The same emoji is a valid comment and refused as an identifier: the classes differ by structure, not by blocklist.
const family=corpus.name[0][1].slice(5);
assert.ok(isComment(family));assert.ok(!isIdentifier(family));assert.equal(visible('a\u0001'+family+'​b',' '),'a '+family+' b');
console.log('PASS unicode corpus: comment, identifier and name classes ('+(corpus.comment.length+corpus.identifier.length+corpus.name.length)+' cases)');

// Server: a comment keeps its emoji, a malformed comment is refused, a store name keeps its emoji.
const sql=new DatabaseSync(':memory:');
for(const f of ['0000_tracker.sql','0001_store_defaults.sql'])sql.exec(readFileSync(new URL('../drizzle/'+f,import.meta.url),'utf8'));
const db={prepare(q){let b=[];return {bind(...v){b=v;return this},async first(){return sql.prepare(q).get(...b)||null},async all(){return {results:sql.prepare(q).all(...b)}},async run(){return {meta:{changes:Number(sql.prepare(q).run(...b).changes)}}}}},async batch(s){sql.exec('BEGIN');try{const out=[];for(const x of s)out.push(await x.run());sql.exec('COMMIT');return out}catch(e){sql.exec('ROLLBACK');throw e}}};
async function call(path,data){const r=await api(new Request('https://app.test/api/'+path,{method:data?'POST':'GET',headers:{'oai-authenticated-user-id':'alice','oai-authenticated-user-email':'alice@example.com',...(data?{origin:'https://app.test','content-type':'application/json'}:{})},body:data?JSON.stringify(data):undefined}),{DB:db});return [r.status,await r.json()]}
const state={items:[],tags:[],people:['Shared'],notes:[],inventory:[],prefs:{},rules:{},recs:{},overrides:{}};
const [code,created]=await call('stores',{name:'Trip '+family,type:'private',initialState:state});
assert.equal(code,201);assert.equal(created.store.name,'Trip '+family);
const sid=created.store.id,note=text=>({id:crypto.randomUUID(),targets:[],text,part:'General',pref:'',at:'2026-01-01T00:00:00Z'});
let [c,r]=await call('stores/'+sid+'/events',{expectedVersion:0,event:{id:'u1',label:'Comment',after:{...state,notes:[note('Love it '+family)]}}});
assert.equal(c,200);assert.equal((await call('stores/'+sid))[1].state.notes[0].text,'Love it '+family);
[c,r]=await call('stores/'+sid+'/events',{expectedVersion:1,event:{id:'u2',label:'Comment',after:{...state,notes:[...(await call('stores/'+sid))[1].state.notes,note('Lone \ud800')]}}});
assert.equal(c,400);assert.match(r.error,/unpaired surrogate/);
[c,r]=await call('stores/'+sid+'/events',{expectedVersion:1,event:{id:'u3',label:'Comment',after:{...state,notes:[...(await call('stores/'+sid))[1].state.notes,note('Bell \u0007')]}}});
assert.equal(c,400);
console.log('PASS unicode server: emoji ZWJ comments and store names kept, malformed comments refused');
