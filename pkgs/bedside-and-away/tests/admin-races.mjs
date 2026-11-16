// Standalone copy of API test harness; deterministic hook interleaves revocation before BEGIN.
import {DatabaseSync} from 'node:sqlite';import {readFileSync} from 'node:fs';import assert from 'node:assert/strict';import {api} from '../worker/api.mjs';
const sql=new DatabaseSync(':memory:');for(const file of ['0000_tracker.sql','0001_store_defaults.sql'])sql.exec(readFileSync(new URL('../drizzle/'+file,import.meta.url),'utf8'));let beforeBatch=null;
const db={prepare(q){let binds=[];return{bind(...v){binds=v;return this},async first(){return sql.prepare(q).get(...binds)||null},async all(){return{results:sql.prepare(q).all(...binds)}},async run(){return{meta:{changes:Number(sql.prepare(q).run(...binds).changes)}}}}},async batch(stmts){if(beforeBatch){const hook=beforeBatch;beforeBatch=null;hook()}sql.exec('BEGIN');try{const out=[];for(const st of stmts)out.push(await st.run());sql.exec('COMMIT');return out}catch(e){sql.exec('ROLLBACK');throw e}}};
async function call(path,data,user='alice'){const r=await api(new Request('https://app.test/api/'+path,{method:data?'POST':'GET',headers:{'oai-authenticated-user-id':user,'oai-authenticated-user-email':user+'@example.com',...(data?{'origin':'https://app.test','content-type':'application/json'}:{})},...(data?{body:JSON.stringify(data)}:{})}),{DB:db});return[r.status,await r.json()]}
const state={items:[],tags:[],people:[],notes:[],inventory:[]};const sid=(await call('stores',{name:'Race',type:'shared',initialState:state}))[1].store.id;const path='stores/'+sid;
sql.prepare("INSERT INTO memberships(store_id,user_id,email,role,created_at) VALUES(?,'bob','bob@example.com','admin','now')").run(sid);
const revoke=()=>sql.prepare("UPDATE memberships SET role='reader' WHERE store_id=? AND user_id='bob'").run(sid);
const restore=()=>sql.prepare("UPDATE memberships SET role='admin' WHERE store_id=? AND user_id='bob'").run(sid);
for(const [route,data,check] of [
 ['settings',{name:'Compromised'},()=>assert.equal(sql.prepare('SELECT name FROM stores WHERE id=?').get(sid).name,'Race')],
 ['members/bob',{role:'admin'},()=>assert.equal(sql.prepare("SELECT role FROM memberships WHERE store_id=? AND user_id='bob'").get(sid).role,'reader')],
 ['invites',{email:'mallory@example.com'},()=>assert.equal(sql.prepare("SELECT COUNT(*) AS n FROM invitations WHERE store_id=?").get(sid).n,0)]
]){restore();const events=sql.prepare('SELECT COUNT(*) AS n FROM access_events').get().n;beforeBatch=revoke;assert.equal((await call(path+'/'+route,data,'bob'))[0],403);check();assert.equal(sql.prepare('SELECT COUNT(*) AS n FROM access_events').get().n,events)}
console.log('PASS admin mutation race: settings, members and invitations cannot outlive revocation or emit false audit.');
const invite=(await call(path+'/invites',{email:'carol@example.com',role:'editor'}))[1].invitation.id;
beforeBatch=()=>sql.prepare("UPDATE stores SET archived=1 WHERE id=?").run(sid);
assert.equal((await call('invites/'+invite+'/respond',{action:'accept'},'carol'))[0],409);
assert.equal(sql.prepare("SELECT COUNT(*) AS n FROM memberships WHERE store_id=? AND user_id='carol'").get(sid).n,0);
sql.prepare('UPDATE stores SET archived=0 WHERE id=?').run(sid);
beforeBatch=()=>sql.prepare("UPDATE invitations SET status='revoked' WHERE id=?").run(invite);
const rejected=await call('invites/'+invite+'/respond',{action:'accept'},'carol');assert.equal(rejected[1].status,'revoked');
assert.equal(sql.prepare("SELECT COUNT(*) AS n FROM memberships WHERE store_id=? AND user_id='carol'").get(sid).n,0);
console.log('PASS invitation response race: archive and revoke prevent accepting/granting membership.');
