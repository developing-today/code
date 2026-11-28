// Backup/restore rehearsal over the full migration set (fixture level).
// Oracle: a logical backup restored into a fresh database preserves store
// IDs, item counts, item placements, note annotations, memberships and
// event versions exactly, stays fully usable through the API, and the
// rehearsal guard refuses to restore an older backup over newer data
// (release.md step 7: never silently overwrite data written after the
// backup). This is a synthetic fixture rehearsal, not a live backup
// rehearsal against the production Turso database.
import {DatabaseSync} from 'node:sqlite';
import {readFileSync} from 'node:fs';
import assert from 'node:assert/strict';
import {api} from '../worker/api.mjs';

const MIGRATIONS=['../drizzle/0000_tracker.sql','../drizzle/0001_store_defaults.sql','../external/0002_gateway.sql','../external/0003_steering_outbox.sql'];
function makeSql(){
  const sql=new DatabaseSync(':memory:');
  for(const rel of MIGRATIONS){
    const text=readFileSync(new URL(rel,import.meta.url),'utf8');
    const parts=(rel.endsWith('0002_gateway.sql')||rel.endsWith('0003_steering_outbox.sql'))?text.split(';'):text.split('--> statement-breakpoint');
    for(const part of parts)if(part.trim())sql.exec(part);
  }
  return sql;
}
function adapter(sql){
  return {prepare(q){let binds=[];return {bind(...v){binds=v;return this},async first(){return sql.prepare(q).get(...binds)||null},async all(){return {results:sql.prepare(q).all(...binds)}},async run(){const r=sql.prepare(q).run(...binds);return {meta:{changes:Number(r.changes)}}}}},async batch(statements){sql.exec('BEGIN');try{const results=[];for(const s of statements)results.push(await s.run());sql.exec('COMMIT');return results}catch(e){sql.exec('ROLLBACK');throw e}}};
}
function caller(db){
  return async(path,{user='alice',email=user+'@example.com',data}={})=>{
    const r=await api(new Request('https://app.test/api/'+path,{method:data?'POST':'GET',headers:{'oai-authenticated-user-id':user,'oai-authenticated-user-email':email,...(data?{'origin':'https://app.test','content-type':'application/json'}:{})},...(data?{body:JSON.stringify(data)}:{})}),{DB:db});
    return [r.status,await r.json()];
  };
}
const base={items:[],tags:[],people:['Shared'],notes:[],inventory:[],prefs:{},rules:{},recs:{},overrides:{}};

// --- Build the source fixture through the public API.
const sourceSql=makeSql(),call=caller(adapter(sourceSql));
let [code,r]=await call('stores',{data:{name:'Household',type:'shared',initialState:base}});
assert.equal(code,201);const sid=r.store.id;
const stocked={...base,
  items:[{id:'g1',kind:'virtual',name:'Lighting',tags:[],attributes:[]},
         {id:'i1',parent:'g1',name:'Restore Lamp',tags:[],attributes:[]},
         {id:'i2',parent:'g1',name:'Spare Bulb',tags:[],attributes:[]}],
  notes:[{id:'n1',targets:['i1'],text:'Check the warranty',part:'General',pref:'',at:'2026-01-01T00:00:00Z'}]};
[code,r]=await call('stores/'+sid+'/events',{data:{expectedVersion:0,event:{id:'ev_1',label:'Stock shelves',after:stocked}}});
assert.equal(code,200);assert.equal(r.version,1);
[code,r]=await call('stores/'+sid+'/invites',{data:{email:'bob@example.com',role:'editor'}});
assert.equal(code,201);const iid=r.invitation.id;
assert.equal((await call('invites/'+iid+'/respond',{user:'bob',data:{action:'accept'}}))[0],200);
const annotated={...stocked,notes:[...stocked.notes,{id:'n2',targets:['i2'],text:'Buy two, one is a spare',part:'General',pref:'',at:'2026-01-02T00:00:00Z'}]};
[code,r]=await call('stores/'+sid+'/events',{user:'bob',data:{expectedVersion:1,event:{id:'ev_2',label:'Annotate bulb',after:annotated}}});
assert.equal(code,200);assert.equal(r.version,2);
[code,r]=await call('stores',{data:{name:'Solo',type:'private',initialState:base}});
assert.equal(code,201);const soloId=r.store.id;
[code,r]=await call('stores/'+soloId+'/events',{data:{expectedVersion:0,event:{id:'ev_solo',label:'Solo item',after:{...base,items:[{id:'s1',name:'Notebook',tags:[],attributes:[]}]}}}});
assert.equal(code,200);assert.equal(r.version,1);

// --- Invariants: the acceptance-criteria view of the data.
function invariants(sql){
  const out={stores:{},counts:{}};
  for(const t of ['stores','events','memberships','invitations','access_events'])out.counts[t]=sql.prepare('SELECT COUNT(*) AS n FROM '+t).get().n;
  for(const s of sql.prepare('SELECT id,type,version,state_json FROM stores ORDER BY id').all()){
    const state=JSON.parse(s.state_json);
    out.stores[s.id]={
      type:s.type,version:s.version,
      itemCount:state.items.length,
      placements:Object.fromEntries(state.items.map(i=>[i.id,i.parent??null])),
      annotations:Object.fromEntries((state.notes||[]).map(n=>[n.id,n.text])),
      eventVersions:sql.prepare('SELECT version FROM events WHERE store_id=? ORDER BY version').all(s.id).map(e=>e.version),
      members:Object.fromEntries(sql.prepare('SELECT user_id,role FROM memberships WHERE store_id=? ORDER BY user_id').all(s.id).map(m=>[m.user_id,m.role]))
    };
  }
  return out;
}
// --- Logical backup: every migration table, rows in creation order.
function dump(sql){
  const tables=sql.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY rowid").all().map(t=>t.name);
  const out={};
  for(const t of tables){
    const cols=sql.prepare('PRAGMA table_info('+t+')').all().map(c=>c.name);
    const st=sql.prepare('SELECT * FROM '+t);st.setReturnArrays(true);
    out[t]={cols,rows:st.all()};
  }
  return out;
}
function restore(sql,backup){
  sql.exec('PRAGMA foreign_keys=OFF');
  for(const [t,{cols,rows}] of Object.entries(backup)){
    const st=sql.prepare('INSERT INTO '+t+' ('+cols.join(',')+') VALUES ('+cols.map(()=>'?').join(',')+')');
    for(const row of rows)st.run(...row);
  }
  sql.exec('PRAGMA foreign_keys=ON');
}
// --- Rollback guard: a backup may never silently overwrite newer data.
function assertRestoreAllowed(backup,targetSql){
  for(const row of backup.stores.rows){
    const id=row[backup.stores.cols.indexOf('id')],version=row[backup.stores.cols.indexOf('version')];
    const current=targetSql.prepare('SELECT version FROM stores WHERE id=?').get(id);
    if(current&&current.version>version)throw Error('Refusing restore of store '+id+': target is at version '+current.version+', backup is at version '+version);
  }
}

const before=invariants(sourceSql),backup=dump(sourceSql);
assert.equal(before.stores[sid].itemCount,3);
assert.deepEqual(before.stores[sid].eventVersions,[1,2]);
assert.deepEqual(before.stores[sid].annotations,{'n1':'Check the warranty','n2':'Buy two, one is a spare'});
assert.equal(before.stores[sid].members.bob,'editor');

// Advance the source past the backup: the guard must refuse the stale restore.
[code,r]=await call('stores/'+sid+'/events',{data:{expectedVersion:2,event:{id:'ev_3',label:'After backup',after:annotated}}});
assert.equal(code,200);assert.equal(r.version,3);
assert.throws(()=>assertRestoreAllowed(backup,sourceSql),/Refusing restore/);
console.log('PASS restore guard: restoring the version-2 backup over the version-3 source is refused, not silently overwritten.');

// Restore into a fresh database and compare everything.
const restoredSql=makeSql();
assertRestoreAllowed(backup,restoredSql);
restore(restoredSql,backup);
assert.deepEqual(invariants(restoredSql),before);
assert.deepEqual(dump(restoredSql),backup);
console.log('PASS restore rehearsal: IDs, counts, placements, annotations, memberships and event versions identical after restore ('+before.counts.stores+' stores, '+before.counts.events+' events, '+before.counts.memberships+' memberships, '+before.counts.invitations+' invitations, '+before.counts.access_events+' access events).');

// The restored database is operational through the API, not just byte-equal.
const callRestored=caller(adapter(restoredSql));
assert.equal((await callRestored('stores/'+sid))[0],200);
assert.equal((await callRestored('stores/'+sid,{user:'bob'}))[0],200);
assert.equal((await callRestored('stores/'+sid,{user:'mallory'}))[0],404);
[code,r]=await callRestored('stores/'+sid+'/events',{user:'bob',data:{expectedVersion:2,event:{id:'ev_restored',label:'Post-restore write',after:annotated}}});
assert.equal(code,200);assert.equal(r.version,3);
console.log('PASS restore rehearsal: restored database serves reads to members, denies non-members, and accepts the next versioned write.');
