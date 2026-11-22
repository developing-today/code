// BA-W01 L2: store roles by their contract names, default store provisioning and the display name.
import {DatabaseSync} from 'node:sqlite';
import {readFileSync} from 'node:fs';
import assert from 'node:assert/strict';
import {api} from '../worker/api.mjs';
import {storedRole,roleName,roleAtLeast,personalStoreName,maySeedDefaultShared,STORED_ROLES} from '../worker/roles.mjs';

for(const [asked,stored] of [['viewer','reader'],['reader','reader'],['editor','editor'],['administrator','admin'],['admin','admin']])assert.equal(storedRole(asked),stored);
for(const x of ['owner','Viewer','remove','','toString','__proto__',null,undefined,1])assert.equal(storedRole(x),null);
assert.deepEqual(STORED_ROLES.map(roleName),['viewer','editor','administrator']);
assert.equal(roleName('owner'),null);assert.equal(roleName('constructor'),null);
assert.ok(roleAtLeast('admin','editor')&&roleAtLeast('editor','editor')&&!roleAtLeast('reader','editor')&&!roleAtLeast('owner','reader'));
assert.equal(personalStoreName(''),'Personal');assert.equal(personalStoreName('Kay'),'Kay personal store');
assert.ok(maySeedDefaultShared({},'a')&&maySeedDefaultShared({DEFAULT_SHARED_OWNER:' '},'a')&&maySeedDefaultShared({DEFAULT_SHARED_OWNER:'a'},'a'));
assert.ok(!maySeedDefaultShared({DEFAULT_SHARED_OWNER:'a'},'b')&&!maySeedDefaultShared({DEFAULT_SHARED_OWNER:'none'},'a'));
console.log('PASS role names, personal store naming and default shared seeding rule');

function harness(env={}){
 const sql=new DatabaseSync(':memory:');
 for(const f of ['0000_tracker.sql','0001_store_defaults.sql'])sql.exec(readFileSync(new URL('../drizzle/'+f,import.meta.url),'utf8'));
 const db={prepare(q){let binds=[];return {bind(...v){binds=v;return this},async first(){return sql.prepare(q).get(...binds)||null},async all(){return {results:sql.prepare(q).all(...binds)}},async run(){const r=sql.prepare(q).run(...binds);return {meta:{changes:Number(r.changes)}}}}},async batch(statements){sql.exec('BEGIN');try{const results=[];for(const s of statements)results.push(await s.run());sql.exec('COMMIT');return results}catch(e){sql.exec('ROLLBACK');throw e}}};
 return async(path,{user='alice',data}={})=>{const r=await api(new Request('https://app.test/api/'+path,{method:data?'POST':'GET',headers:{'oai-authenticated-user-id':user,'oai-authenticated-user-email':user+'@example.com',...(data?{origin:'https://app.test','content-type':'application/json'}:{})},...(data?{body:JSON.stringify(data)}:{})}),{...env,DB:db});return [r.status,await r.json()]};
}
const state={items:[],tags:[],people:['Shared'],notes:[],inventory:[],prefs:{},rules:{},recs:{},overrides:{}};

// Configured owner: only that account seeds the default shared store, whoever provisions first.
{
 const call=harness({DEFAULT_SHARED_OWNER:'carol'});
 let [code,r]=await call('provision',{user:'bob',data:{initialState:state}});
 assert.equal(code,200);assert.ok(r.personalStoreId);assert.equal(r.defaultSharedStoreId,null);
 assert.deepEqual(r.stores.map(s=>s.id),[r.personalStoreId]);
 [code,r]=await call('provision',{user:'carol',data:{initialState:state}});
 assert.equal(code,200);assert.ok(r.defaultSharedStoreId);
 const shared=r.defaultSharedStoreId;
 assert.equal(r.stores.find(s=>s.id===shared).roleName,'administrator');
 [code,r]=await call('provision',{user:'bob',data:{initialState:state}});
 assert.equal(r.defaultSharedStoreId,null,'provisioning never joins another account to the shared store');
 assert.equal((await call('stores/'+shared,{user:'bob'}))[0],404);
 // Contract role names are accepted on invitations and role changes; the stored names are kept.
 [code,r]=await call('stores/'+shared+'/invites',{user:'carol',data:{email:'bob@example.com',role:'viewer'}});
 assert.equal(code,201);assert.equal(r.invitation.role,'reader');assert.equal(r.invitation.roleName,'viewer');
 assert.equal((await call('invites/'+r.invitation.id+'/respond',{user:'bob',data:{action:'accept'}}))[0],200);
 [code,r]=await call('stores/'+shared,{user:'bob'});assert.equal(code,200);assert.equal(r.store.role,'reader');assert.equal(r.store.roleName,'viewer');
 assert.equal((await call('stores/'+shared+'/members/bob',{user:'carol',data:{role:'administrator'}}))[0],200);
 [code,r]=await call('stores/'+shared+'/access',{user:'bob'});assert.equal(code,200);
 assert.deepEqual(r.members.map(m=>[m.userId,m.role,m.roleName]).sort(),[['bob','admin','administrator'],['carol','admin','administrator']]);
 assert.equal(r.invitations[0].roleName,'viewer');
 for(const role of ['owner','Viewer','',null])assert.equal((await call('stores/'+shared+'/members/bob',{user:'carol',data:{role}}))[0],400);
 assert.equal((await call('stores/'+shared+'/invites',{user:'carol',data:{email:'dan@example.com',role:'owner'}}))[0],400);
 assert.equal((await call('stores/'+shared+'/members/bob',{user:'carol',data:{role:'remove'}}))[0],200);
 assert.equal((await call('stores/'+shared,{user:'bob'}))[0],404);
}
// `none`: no account seeds a shared store; each still gets one personal store.
{
 const call=harness({DEFAULT_SHARED_OWNER:'none'});
 const [code,r]=await call('provision',{data:{initialState:state}});
 assert.equal(code,200);assert.equal(r.defaultSharedStoreId,null);assert.equal(r.stores.length,1);assert.equal(r.stores[0].type,'private');
}
// Unset keeps the prototype rule: the first account seeds it, later accounts do not join.
{
 const call=harness();
 const first=(await call('provision',{data:{initialState:state}}))[1];
 assert.ok(first.defaultSharedStoreId);
 assert.equal((await call('provision',{user:'bob',data:{initialState:state}}))[1].defaultSharedStoreId,null);
}
console.log('PASS default shared store seeded only by its configured owner, never joined by provisioning; contract role names at the API');

// The personal store's automatic name follows the display name; a chosen name stays; the ID never changes.
{
 const call=harness();
 let r=(await call('provision',{data:{initialState:state}}))[1];
 const personal=r.personalStoreId,named=()=>call('bootstrap').then(([,b])=>b.stores.find(s=>s.id===personal));
 assert.equal((await named()).name,'Personal');
 await call('settings',{data:{nickname:'Kay'}});
 assert.equal((await named()).name,'Kay personal store');
 await call('settings',{data:{nickname:'Kay R'}});
 assert.equal((await named()).name,'Kay R personal store');
 await call('settings',{data:{nickname:''}});
 assert.equal((await named()).name,'Personal');
 assert.equal((await call('stores/'+personal+'/settings',{data:{name:'My notes'}}))[0],200);
 await call('settings',{data:{nickname:'Lee'}});
 const kept=await named();assert.equal(kept.name,'My notes');assert.equal(kept.id,personal);
 assert.equal((await call('bootstrap'))[1].personalStoreId,personal);
 // Another account's nickname never renames this store.
 await call('provision',{user:'bob',data:{initialState:state}});
 await call('settings',{user:'bob',data:{nickname:'Personal'}});
 assert.equal((await named()).name,'My notes');
 // Settings without a nickname change leave the name alone.
 await call('stores/'+personal+'/settings',{data:{name:'Lee personal store'}});
 await call('settings',{data:{showHidden:true}});
 assert.equal((await named()).name,'Lee personal store');
}
console.log('PASS display name renames the personal store label only, never its identifier, and never a chosen name');
