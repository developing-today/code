// BA-W01 §8 tests 4 and 8 (L8). Test 4: every terminal invitation state refuses every response, and the
// refusal changes nothing. Test 8: moving an entry inside a store (category, group, order) never
// changes who can see it, and never leaks a concealed id through the moved entry's links.
import {DatabaseSync} from 'node:sqlite';
import {readFileSync} from 'node:fs';
import assert from 'node:assert/strict';
import {api} from '../worker/api.mjs';
import {projectSnapshot} from '../worker/access.mjs';

const sql=new DatabaseSync(':memory:');
for(const f of ['0000_tracker.sql','0001_store_defaults.sql'])sql.exec(readFileSync(new URL('../drizzle/'+f,import.meta.url),'utf8'));
const db={prepare(q){let binds=[];return {bind(...v){binds=v;return this},async first(){return sql.prepare(q).get(...binds)||null},async all(){return {results:sql.prepare(q).all(...binds)}},async run(){const r=sql.prepare(q).run(...binds);return {meta:{changes:Number(r.changes)}}}}},async batch(statements){sql.exec('BEGIN');try{const results=[];for(const s of statements)results.push(await s.run());sql.exec('COMMIT');return results}catch(e){sql.exec('ROLLBACK');throw e}}};

const call=async(path,{user='alice',data}={})=>{const r=await api(new Request('https://app.test/api/'+path,{method:data?'POST':'GET',headers:{'oai-authenticated-user-id':user,'oai-authenticated-user-email':user+'@example.com',...(data?{origin:'https://app.test','content-type':'application/json'}:{})},...(data?{body:JSON.stringify(data)}:{})}),{DB:db});return [r.status,await r.json()]};

// §8 test 4: the transition table. Oracle: from accepted, declined, revoked or expired, both accept and
// decline answer 409 with the current status, and membership, the invitation row and the audit log are
// unchanged.
const state={items:[],tags:[],people:[],notes:[],inventory:[],prefs:{},rules:{},recs:{},overrides:{}};
const sid=(await call('stores',{data:{name:'Invites',type:'shared',initialState:state}}))[1].store.id;
const invite=async user=>{const [code,r]=await call('stores/'+sid+'/invites',{data:{email:user+'@example.com',role:'reader'}});assert.equal(code,201);return r.invitation.id};
const terminal={};
terminal.accepted=await invite('erin');assert.equal((await call('invites/'+terminal.accepted+'/respond',{user:'erin',data:{action:'accept'}}))[0],200);
terminal.declined=await invite('fay');assert.equal((await call('invites/'+terminal.declined+'/respond',{user:'fay',data:{action:'decline'}}))[0],200);
terminal.revoked=await invite('gus');assert.equal((await call('stores/'+sid+'/invites/'+terminal.revoked+'/revoke',{data:{}}))[0],200);
terminal.expired=await invite('hal');sql.prepare('UPDATE invitations SET created_at=? WHERE id=?').run(new Date(Date.now()-15*86400000).toISOString(),terminal.expired);
const owner={accepted:'erin',declined:'fay',revoked:'gus',expired:'hal'};
const snapshot=()=>JSON.stringify([sql.prepare('SELECT * FROM memberships ORDER BY store_id,user_id').all(),sql.prepare('SELECT * FROM invitations ORDER BY id').all(),sql.prepare('SELECT COUNT(*) AS n FROM access_events').get().n]);
let cells=0;
for(const [status,id] of Object.entries(terminal))for(const action of ['accept','decline']){
 const before=snapshot();
 const [code,r]=await call('invites/'+id+'/respond',{user:owner[status],data:{action}});
 assert.equal(code,409,status+' x '+action);assert.equal(r.status,status,status+' x '+action);
 assert.equal(snapshot(),before,status+' x '+action+' must change nothing');cells++;
}
// Only the accepted invitee became a member; the others still cannot see the store.
assert.equal((await call('stores/'+sid,{user:'erin'}))[0],200);
for(const user of ['fay','gus','hal'])assert.equal((await call('stores/'+sid,{user}))[0],404);
// Someone else presenting an invitation id answers as if it doesn't exist.
assert.equal((await call('invites/'+terminal.expired+'/respond',{user:'mallory',data:{action:'accept'}}))[0],404);
console.log('PASS §8.4: '+cells+' terminal-state x response cells refuse with the current status and change nothing.');

// §8 test 8: moves never broaden visibility. Oracle: for generated stores, policies and principals, the
// visible id set before and after a move is identical, and no visible entry links to a concealed id.
let seed=0x2545f491;const rand=n=>{seed^=seed<<13;seed^=seed>>>17;seed^=seed<<5;return (seed>>>0)%n};
const pick=xs=>xs[rand(xs.length)];
const users=['alice','bob','carol'],roles=['reader','editor','admin'];
function store(){
 const tags=Array.from({length:3},(_,i)=>({id:'t'+i,name:'Tag '+i}));
 const items=Array.from({length:6},(_,i)=>({id:'i'+i,name:'Item '+i,kind:i<2?'virtual':'item',tags:[],parent:'',order:i}));
 for(const it of items){if(rand(2))it.tags=[pick(tags).id];if(it.kind!=='virtual'&&rand(2))it.parent=pick(['i0','i1'])}
 const policies={};const groups={g:users.filter(()=>rand(2))};
 const subject=()=>pick([{type:'everyone'},{type:'user',id:pick(users)},{type:'group',id:'g'}]);
 for(const id of ['*',...items.map(x=>'item:'+x.id),...tags.map(x=>'tag:'+x.id)])if(id==='*'||rand(3)===0)policies[id]={...(rand(4)===0?{default:'deny'}:{}),rules:Array.from({length:rand(3)},()=>({effect:pick(['allow','deny']),subject:subject(),actions:['read']}))};
 if(!policies['*'].rules.length&&!policies['*'].default)policies['*'].rules.push({effect:'allow',subject:{type:'everyone'},actions:['read']});
 return {...state,tags,items,access:{schemaVersion:1,groups,policies}};
}
function move(s){
 const out=structuredClone(s),it=pick(out.items.filter(x=>x.kind!=='virtual'));
 const kind=rand(3);
 if(kind===0)it.parent=pick(['','i0','i1']);
 else if(kind===1)it.tags=[pick(out.tags).id];
 else{it.order=-1;out.items.sort((a,b)=>a.order-b.order)}
 return out;
}
const visible=(s,principal,role)=>{const v=projectSnapshot(s,{principal:{id:principal},role});return {items:new Set(v.items.map(x=>x.id)),tags:new Set(v.tags.map(x=>x.id)),view:v}};
let checks=0;
for(let n=0;n<3000;n++){
 const s=store(),moved=move(s);
 for(const principal of users)for(const role of roles){
  const a=visible(s,principal,role),b=visible(moved,principal,role);
  assert.deepEqual([...b.items].sort(),[...a.items].sort(),'a move changed which entries are visible');
  assert.deepEqual([...b.tags].sort(),[...a.tags].sort(),'a move changed which categories are visible');
  for(const it of b.view.items){
   assert.ok(it.parent===''||b.items.has(it.parent),'moved entry links to a concealed group');
   assert.ok((it.tags||[]).every(t=>b.tags.has(t)),'moved entry links to a concealed category');
  }
  checks++;
 }
}
console.log('PASS §8.8: '+checks+' principal x role checks over 3000 generated moves; visibility never changed.');
