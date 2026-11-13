import {historyTransition} from './history.mjs';
import {evaluateAccess,projectSnapshot,projectEvents,assertSnapshotMutation,validateAccessConfig,hasScopedAccess} from './access.mjs';
import {transitionSteering,assertSteeringUnchanged} from './steering.mjs';
import {validateVariantGraph,assertVariantMetadataUnchanged} from './variant-graph.mjs';
import {variantPlan} from './variant-plan.mjs';
const roles = ['reader','editor','admin'];
const headers = {'content-type':'application/json; charset=utf-8','cache-control':'no-store','x-content-type-options':'nosniff'};
const json = (body,status=200)=>new Response(JSON.stringify(body),{status,headers});
const fail = (status,message)=>{throw Object.assign(new Error(message),{status})};
const clean = (x,max=200)=>String(x??'').normalize('NFC').replace(/[\p{Cc}\p{Cf}]/gu,'').trim().slice(0,max);
const ident = x=>typeof x==='string'&&/^[A-Za-z0-9_-]{1,100}$/.test(x);
const parse = x=>JSON.parse(x);
const now = ()=>new Date().toISOString();
const roleValue = x=>{if(!roles.includes(x))fail(400,'Invalid role');return x};
const canonical = value=>JSON.stringify(value,(_,v)=>v&&typeof v==='object'&&!Array.isArray(v)?Object.fromEntries(Object.keys(v).sort().map(k=>[k,v[k]])):v);
const publicEvent = e=>{const {intent,requestResult,...visible}=e;return visible};
const eventIntent = (event,expectedVersion)=>({expectedVersion,after:event.after,label:clean(event.label,300)||'Edited tracker',operation:clean(event.operation,32)||'change',target:clean(event.target,100)});
function retryResult(prior,sid,user,intent){
 const accepted=parse(prior.payload_json);
 if(prior.store_id!==sid||accepted.actorId!==user.id||!accepted.intent||canonical(accepted.intent)!==canonical(intent))fail(409,'Command ID belongs to a different actor or intent');
 return {version:prior.version,event:accepted,duplicate:true};
}
function validateState(s) {
 if(!s || typeof s!=='object' || Array.isArray(s))fail(400,'State must be an object');
 for(const k of ['items','tags','notes','inventory','people'])if(!Array.isArray(s[k]))fail(400,'Missing state array: '+k);
 if(s.items.length>10000 || s.tags.length>2000)fail(413,'Store is too large');
 for(const item of s.items)if(!item||typeof item.id!=='string'||typeof item.name!=='string'||!Array.isArray(item.tags)||!Array.isArray(item.attributes))fail(400,'Invalid item');
 if(new Set(s.items.map(x=>x.id)).size!==s.items.length)fail(400,'Duplicate item IDs');
 if(new Set(s.tags.map(x=>x?.id)).size!==s.tags.length)fail(400,'Duplicate tag IDs');
 if(JSON.stringify(s).length>1500000)fail(413,'Store snapshot limit is 1.5 MB; split the store or archive/export old content');
 validateAccessConfig(s.access);
 try{validateVariantGraph(s.items)}catch(e){fail(400,e.message)}
 return s;
}
function validateNewRecords(before,after,user,name){
 for(const field of ['notes','inventory']){
  const previous=new Set(before[field].map(x=>x.id));
  for(const row of after[field])if(!previous.has(row.id)){
   if(!ident(row.id)&&!(typeof row.id==='string'&&/^receipt:[A-Za-z0-9_:-]{1,190}$/.test(row.id)))fail(400,'Record ID required');
   row.actorId=user.id;row.author=name;row.recordedAt=now();
   if(field==='inventory'){
    if(!['Bought','Consumed','Stock adjustment','Expense'].includes(row.mode)||!Number.isFinite(row.qty)||(row.mode!=='Stock adjustment'&&row.qty<0))fail(400,'Invalid inventory quantity or mode');
    if(row.totalMinor!==undefined&&row.totalMinor!==null&&(!Number.isSafeInteger(row.totalMinor)||row.totalMinor<0))fail(400,'Invalid integer minor-unit total');
    if(row.total!==undefined&&row.total!==null&&(!Number.isFinite(row.total)||row.total<0))fail(400,'Invalid expense total');
    if(row.allocations!=null){const a=row.allocations;if(!Array.isArray(a)||!a.length||!Number.isSafeInteger(row.totalMinor)||a.some(x=>!x||typeof x.categoryId!=='string'||!Number.isSafeInteger(x.minor)||x.minor<0)||new Set(a.map(x=>x.categoryId)).size!==a.length||a.reduce((n,x)=>n+x.minor,0)!==row.totalMinor)fail(400,'Expense allocations must sum to total');}
   }
  }
 }
}
async function body(req){
 const raw=await req.text();if(raw.length>1700000)fail(413,'Request too large');
 try{return JSON.parse(raw)}catch{fail(400,'Invalid JSON')}
}
function storeData(row){return {id:row.id,name:row.name,type:row.is_open?'open':row.type,ownerId:row.owner_id,archived:!!row.archived,version:row.version,role:row.role,createdAt:row.created_at,updatedAt:row.updated_at}}
const settingsDefault={nickname:'',showHidden:false,showArchived:false,showHiddenStores:false,stores:{}};
async function getSettings(db,user){const r=await db.prepare('SELECT settings_json FROM user_settings WHERE user_id=?').bind(user.id).first();return {...settingsDefault,...(r?parse(r.settings_json):{})}}
async function access(db,id,user,level='reader',allowArchived=true){
 if(!ident(id))fail(404,'Store not found');
 const s=await db.prepare("SELECT s.*,m.role AS member_role,COALESCE(m.role,CASE WHEN s.is_open=1 THEN 'reader' END) AS role FROM stores s LEFT JOIN memberships m ON m.store_id=s.id AND m.user_id=? WHERE s.id=? AND (s.is_open=1 OR m.user_id IS NOT NULL)").bind(user.id,id).first();
 if(!s||(s.is_open&&s.archived&&!s.member_role)||(!s.is_open&&s.type==='private'&&s.owner_id!==user.id))fail(404,'Store not found');
 if(roles.indexOf(s.role)<roles.indexOf(level))fail(403,'Insufficient store role');
 if(!allowArchived&&s.archived)fail(409,'Restore the archived store before editing');
 return s;
}
function audit(db,id,user,action,details){return db.prepare('INSERT INTO access_events(id,store_id,actor_id,at,action,details_json) VALUES(?,?,?,?,?,?)').bind(crypto.randomUUID(),id,user.id,now(),action,JSON.stringify(details))}
async function adminMutation(db,sid,user,action,details,makeWrites,{activeShared=false}={}){
 const receipt=crypto.randomUUID();
 const guard=db.prepare("INSERT INTO access_events(id,store_id,actor_id,at,action,details_json) SELECT ?,s.id,?,?,?,? FROM stores s JOIN memberships m ON m.store_id=s.id WHERE s.id=? AND m.user_id=? AND m.role='admin' AND (s.type='shared' OR s.owner_id=?) AND (?=0 OR (s.archived=0 AND s.type='shared'))").bind(receipt,user.id,now(),action,JSON.stringify(details),sid,user.id,user.id,+activeShared);
 const results=await db.batch([guard,...makeWrites(receipt)]);
 if(!results[0]?.meta?.changes)fail(403,'Administrator authority changed; reload before retrying');
}
function publicSnapshot(state){
 const tags=state.tags.filter(t=>!t.archived&&!t.hidden),allowed=new Set(tags.map(t=>t.id));
 const items=state.items.filter(i=>!i.archived&&!i.hidden).map(i=>({...i,tags:i.tags.filter(t=>allowed.has(t))}));
 const ids=new Set(items.map(i=>i.id));
 return {people:state.people,prefs:{},items:items.map(i=>({...i,parent:ids.has(i.parent)?i.parent:''})),tags,notes:[],inventory:[],rules:{},overrides:{},recs:{},budget:0,nickname:''};
}
async function accessible(db,user){
 return (await db.prepare("SELECT s.*,m.role AS member_role,COALESCE(m.role,CASE WHEN s.is_open=1 THEN 'reader' END) AS role FROM stores s LEFT JOIN memberships m ON m.store_id=s.id AND m.user_id=? WHERE (s.is_open=1 AND (s.archived=0 OR m.user_id IS NOT NULL)) OR (m.user_id IS NOT NULL AND (s.type='shared' OR s.owner_id=?)) ORDER BY s.created_at").bind(user.id,user.id).all()).results;
}
async function bootstrap(db,user){
 const stores=(await accessible(db,user)).map(storeData);
 const invitations=user.email?(await db.prepare("SELECT i.id,i.store_id AS storeId,i.role,i.status,i.created_at AS createdAt,s.name AS storeName FROM invitations i JOIN stores s ON s.id=i.store_id WHERE i.email=? AND i.status='pending' AND s.type='shared' AND s.archived=0 ORDER BY i.created_at").bind(user.email).all()).results:[];
 const refs=(await db.prepare("SELECT key,store_id FROM default_stores WHERE key IN (?, 'shared')").bind('personal:'+user.id).all()).results;
 const ref=k=>{const found=refs.find(r=>r.key===k)?.store_id;return stores.some(s=>s.id===found)?found:null};
 return {user,settings:await getSettings(db,user),stores,invitations,personalStoreId:ref('personal:'+user.id),defaultSharedStoreId:ref('shared')};
}
export async function api(request,env){
 try{
  const url=new URL(request.url),path=url.pathname.split('/').filter(Boolean),method=request.method;
  if(path[0]!=='api')return json({error:'Not found'},404);
  const id=request.headers.get('oai-authenticated-user-id'),email=clean(request.headers.get('oai-authenticated-user-email'),320).toLowerCase();
  if(!id)return json({error:'Sign in with ChatGPT to use your private database',signIn:'/signin-with-chatgpt?returnTo=/'},401);
  const user={id,email};
  if(!env.DB)fail(503,'Dedicated database binding is not available');
  const db=env.DB;
  if(!['GET','POST'].includes(method))fail(405,'Method not allowed');
  if(method==='POST'){
   const origin=request.headers.get('origin');
   if(origin!==url.origin)fail(403,'Same-origin browser request required');
   if(request.headers.get('sec-fetch-site')==='cross-site')fail(403,'Cross-site writes are blocked');
   if(!request.headers.get('content-type')?.startsWith('application/json'))fail(415,'JSON content type required');
  }
  if(path[1]==='bootstrap'&&method==='GET')return json(await bootstrap(db,user));
  if(path[1]==='provision'&&method==='POST'){
   const b=await body(request),state=validateState(b.initialState),at=now();
   const digest=await crypto.subtle.digest('SHA-256',new TextEncoder().encode(id));
   const personal='personal_'+Array.from(new Uint8Array(digest)).map(x=>x.toString(16).padStart(2,'0')).join('');
   const settings=await getSettings(db,user),name=(settings.nickname?settings.nickname+' personal store':'Personal');
   const shared='site_default_shared';
   const encoded=JSON.stringify(state);
   const personalState=JSON.stringify({...state,items:[],notes:[],inventory:[],research:[],rules:{},overrides:{},recs:{}});
   await db.batch([
    db.prepare("INSERT OR IGNORE INTO stores(id,name,type,owner_id,state_json,created_at,updated_at) VALUES(?,?,'private',?,?,?,?)").bind(personal,name,id,personalState,at,at),
    db.prepare("INSERT OR IGNORE INTO memberships(store_id,user_id,email,role,created_at) SELECT id,?,?,'admin',? FROM stores WHERE id=? AND owner_id=?").bind(id,email,at,personal,id),
    db.prepare("INSERT OR IGNORE INTO default_stores(key,store_id) VALUES(?,?)").bind('personal:'+id,personal),
    db.prepare("INSERT OR IGNORE INTO stores(id,name,type,owner_id,state_json,created_at,updated_at) VALUES(?,'Shared household','shared',?,?,?,?)").bind(shared,id,encoded,at,at),
    db.prepare("INSERT OR IGNORE INTO memberships(store_id,user_id,email,role,created_at) SELECT id,?,?,'admin',? FROM stores WHERE id=? AND owner_id=?").bind(id,email,at,shared,id),
    db.prepare("INSERT OR IGNORE INTO default_stores(key,store_id) VALUES('shared',?)").bind(shared)
   ]);
   return json(await bootstrap(db,user));
  }
  if(path[1]==='search'&&method==='GET'){
   const query=clean(url.searchParams.get('q'),200).toLocaleLowerCase();
   if(!query)return json({query,results:[],truncated:false});
   const settings=await getSettings(db,user),all=await accessible(db,user),results=[];
   for(const store of all){
    if(store.archived||settings.stores?.[store.id]?.hidden)continue;
    const state=projectSnapshot(parse(store.state_json),{principal:user,role:store.role,openVisitor:!!(store.is_open&&!store.member_role)});
    const hits=state.items.filter(item=>!item.archived&&!item.hidden&&[item.name,item.description,...(item.attributes||[]).map(([k,v])=>k+' '+JSON.stringify(v))].join(' ').toLocaleLowerCase().includes(query));
    for(const item of hits){results.push({storeId:store.id,storeName:store.name,itemId:item.id,name:item.name,kind:item.kind||'item',description:clean(item.description,500)});if(results.length>100)return json({query,results:results.slice(0,100),truncated:true})}
   }
   return json({query,results,truncated:false});
  }
  if(path[1]==='settings'&&method==='POST'){
   const b=await body(request),old=await getSettings(db,user);
   const s={...old};
   if('nickname'in b)s.nickname=clean(b.nickname,80);
   for(const k of ['showHidden','showArchived','showHiddenStores'])if(k in b)s[k]=!!b[k];
   if(b.stores&&typeof b.stores==='object'&&!Array.isArray(b.stores))s.stores=JSON.parse(JSON.stringify(b.stores));
   if(JSON.stringify(s).length>50000)fail(413,'Settings too large');
   await db.prepare('INSERT INTO user_settings(user_id,settings_json,updated_at) VALUES(?,?,?) ON CONFLICT(user_id) DO UPDATE SET settings_json=excluded.settings_json,updated_at=excluded.updated_at').bind(id,JSON.stringify(s),now()).run();
   return json({settings:s});
  }
  if(path[1]==='stores'&&path.length===2&&method==='POST'){
   const b=await body(request),name=clean(b.name,160),type=b.type||'private';
   if(!name||!['private','shared','open'].includes(type))fail(400,'Name and private/shared/open store type required');
   let state=validateState(b.initialState);if(type==='open')state={...state,items:[],tags:[],notes:[],inventory:[],research:[],rules:{},overrides:{},recs:{}};
   const sid=crypto.randomUUID(),at=now();
   await db.batch([
    db.prepare('INSERT INTO stores(id,name,type,is_open,owner_id,state_json,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)').bind(sid,name,type==='open'?'shared':type,+(type==='open'),id,JSON.stringify(state),at,at),
    db.prepare('INSERT INTO memberships(store_id,user_id,email,role,created_at) VALUES(?,?,?,?,?)').bind(sid,id,email,'admin',at),
    audit(db,sid,user,'store.created',{name,type})
   ]);
   return json({store:storeData(await access(db,sid,user)),state,version:0},201);
  }
  if(path[1]==='invites'&&path[3]==='respond'&&method==='POST'){
   const b=await body(request),iid=path[2];if(!ident(iid)||!['accept','decline'].includes(b.action))fail(400,'Invalid invitation response');
   const i=await db.prepare('SELECT i.*,s.type,s.archived FROM invitations i JOIN stores s ON s.id=i.store_id WHERE i.id=? AND i.email=?').bind(iid,email).first();
   if(!email||!i)fail(404,'Invitation not found for this signed-in account');
   if(i.status!=='pending')return json({status:i.status});
   if(b.action==='accept'&&(i.type!=='shared'||i.archived))fail(409,'This store is not currently accepting members');
   const at=now(),status=b.action==='accept'?'accepted':'declined';
   const receipt=crypto.randomUUID();
   const stmts=[db.prepare("INSERT INTO access_events(id,store_id,actor_id,at,action,details_json) SELECT ?,i.store_id,?,?,?,? FROM invitations i JOIN stores s ON s.id=i.store_id WHERE i.id=? AND i.email=? AND i.status='pending' AND (?='decline' OR (s.type='shared' AND s.archived=0))").bind(receipt,id,at,'invite.'+status,JSON.stringify({inviteId:iid,email}),iid,email,b.action)];
   if(b.action==='accept')stmts.push(db.prepare("INSERT INTO memberships(store_id,user_id,email,role,created_at) SELECT store_id,?,?,role,? FROM invitations WHERE id=? AND status='pending' AND EXISTS(SELECT 1 FROM access_events WHERE id=?) ON CONFLICT(store_id,user_id) DO NOTHING").bind(id,email,at,iid,receipt));
   stmts.push(db.prepare("UPDATE invitations SET status=?,responded_at=? WHERE id=? AND status='pending' AND EXISTS(SELECT 1 FROM access_events WHERE id=?)").bind(status,at,iid,receipt));
   const results=await db.batch(stmts);
   if(!results[0]?.meta?.changes){const actual=await db.prepare('SELECT status FROM invitations WHERE id=? AND email=?').bind(iid,email).first();if(actual&&actual.status!=='pending')return json({status:actual.status,storeId:i.store_id});fail(409,'Invitation or store changed; reload before responding');}
   return json({status,storeId:i.store_id});
  }
  if(path[1]!=='stores'||!path[2])fail(404,'Route not found');
  const sid=path[2],sub=path[3];
  if(path.length===3&&method==='GET'){
   const s=await access(db,sid,user);
   const rows=s.is_open&&!s.member_role?{results:[]}:await db.prepare('SELECT payload_json FROM events WHERE store_id=? ORDER BY version').bind(sid).all();
   const source=parse(s.state_json),context={principal:user,role:s.role,openVisitor:!!(s.is_open&&!s.member_role)};
   return json({snapshotWritable:!hasScopedAccess(source)&&['editor','admin'].includes(s.role)&&!s.archived,store:storeData(s),state:projectSnapshot(source,context),version:s.version,events:projectEvents(rows.results.map(x=>publicEvent(parse(x.payload_json))),source,context)});
  }
  if(sub==='policy'&&method==='GET'){
   const s=await access(db,sid,user,'admin');
   const members=await db.prepare('SELECT user_id AS userId,email,role FROM memberships WHERE store_id=?').bind(sid).all();
   return json({access:parse(s.state_json).access??{schemaVersion:1,groups:{},policies:{}},version:s.version,members:members.results});
  }
  if(['events','steering','variants','policy','history'].includes(sub)&&method==='POST'){
   const s=await access(db,sid,user,'editor',false),b=await body(request),source=parse(s.state_json),context={principal:user,role:s.role};
   let event=b.event,requestResult;
   const dedicated=sub!=='events';
   const command=sub==='history'?{commandId:b.commandId,action:b.action,targetEventId:b.targetEventId}:sub==='policy'?{commandId:b.commandId,access:b.access}:sub==='variants'?{commandId:b.commandId,ids:b.ids,options:b.options}:b.command;
   const commandId=command?.commandId;
   const intentDedicated={kind:sub,expectedVersion:b.expectedVersion,command};
   if(dedicated){
    if(!ident(commandId)||!Number.isInteger(b.expectedVersion)||b.expectedVersion<0)fail(400,'Command ID and expectedVersion required');
    const prior=await db.prepare('SELECT store_id,version,payload_json FROM events WHERE id=?').bind(commandId).first();
    if(prior){const replay=retryResult(prior,sid,user,intentDedicated);return json({version:replay.version,duplicate:true});}
    if(s.version!==b.expectedVersion)fail(409,'Store changed; reload before applying command');
    if(sub==='policy'){
     if(s.role!=='admin')fail(403,'Administrator required');validateAccessConfig(b.access);
     for(const entityId of ['*',...source.items.map(x=>'item:'+x.id),...source.tags.map(x=>'tag:'+x.id),...source.notes.filter(x=>x.id).map(x=>'note:'+x.id),...source.inventory.filter(x=>x.id).map(x=>'inventory:'+x.id)])for(const action of ['read','write','admin'])for(const principal of [user,{id:s.owner_id}])if(!evaluateAccess({...context,principal,access:b.access,entityId,action}).allowed)fail(400,'Policy would lock out an administrator or owner');
     event={id:commandId,after:{...source,access:b.access},label:'Updated object access policy',operation:'policy'};
    }else if(sub==='steering'){
     if(hasScopedAccess(source))fail(403,'Scoped steering requires a request visibility policy; unavailable until configured');
     if(!evaluateAccess({...context,access:source.access,action:'write',entityId:'*'}).allowed)fail(403,'Write access required');
     const result=transitionSteering({state:source,storeId:sid,storeVersion:s.version,actor:{id:user.id,name:user.email},role:s.role,command,canAccessTarget:t=>['read','write'].every(action=>evaluateAccess({...context,access:source.access,action,entityId:(t.kind==='category'?'tag':t.kind)+':'+t.id}).allowed)});
     requestResult=result.request;
     if(result.duplicate)return json({version:s.version,duplicate:true});
     event={id:commandId,after:result.state,label:'Request '+command.action,operation:'steering',target:command.requestId};
    }else if(sub==='history'){
     if(hasScopedAccess(source))fail(403,'Scoped history edits are unavailable');
     assertSnapshotMutation(source,source,context);
     if(!ident(b.targetEventId)||!['undo','redo'].includes(b.action))fail(400,'History action and targetEventId required');
     const original=await db.prepare('SELECT payload_json FROM events WHERE id=? AND store_id=?').bind(b.targetEventId,sid).first();
     if(!original)fail(404,'Historical event not found');
     const last=await db.prepare("SELECT payload_json FROM events WHERE store_id=? AND json_extract(payload_json,'$.target')=? AND json_extract(payload_json,'$.operation') IN ('history.undo','history.redo') ORDER BY version DESC LIMIT 1").bind(sid,b.targetEventId).first();
     const next=historyTransition({current:source,event:parse(original.payload_json),action:b.action,latest:last?parse(last.payload_json):null});
     event={id:commandId,after:next,label:(b.action==='undo'?'Undo: ':'Redo: ')+parse(original.payload_json).label,operation:'history.'+b.action,target:b.targetEventId};
    }else{
     assertSnapshotMutation(source,source,context);
     const next=structuredClone(source);
     try{variantPlan(next,b.ids,b.options,()=>crypto.randomUUID())}catch(e){fail(400,e.message)}
     event={id:commandId,after:next,label:'Variant '+b.options?.action,operation:'variants'};
    }
   }
   if(!event||!ident(event.id)||!Number.isInteger(b.expectedVersion)||b.expectedVersion<0)fail(400,'Event ID and expectedVersion required');
   const prior=await db.prepare('SELECT store_id,version,payload_json FROM events WHERE id=?').bind(event.id).first();
   const intent=dedicated?intentDedicated:eventIntent(event,b.expectedVersion);
   if(prior){const replay=retryResult(prior,sid,user,intent);return json({version:replay.version,duplicate:true,...(!hasScopedAccess(source)&&!hasScopedAccess(replay.event.before)&&!hasScopedAccess(replay.event.after)?{event:publicEvent(replay.event)}:{})});}
   if(s.version!==b.expectedVersion)return json({error:'Store changed on another device. Reload and merge your edit.',version:s.version},409);
   if(!dedicated){if(source.access&&!hasScopedAccess(source)&&!Object.hasOwn(event.after??{},'access'))event.after={...event.after,access:source.access};if(event.operation&&event.operation!=='change')fail(400,'Use the dedicated command endpoint');if(canonical(source.access??null)!==canonical(event.after?.access??null))fail(403,'Use the administrator policy endpoint');if(hasScopedAccess(source))fail(403,'Scoped stores require per-entity commands; snapshot editing is read-only');assertSnapshotMutation(source,event.after,context);assertSteeringUnchanged(source,event.after);try{assertVariantMetadataUnchanged(source.items,event.after.items)}catch(e){fail(400,e.message)}}
   const after=validateState(structuredClone(event.after)),settings=await getSettings(db,user),version=s.version+1;
   if(sub!=='history')validateNewRecords(source,after,user,settings.nickname||email||'Signed-in user');
   const accepted={intent,...(requestResult?{requestResult}:{}),id:event.id,at:now(),actor:settings.nickname||email||'Signed-in user',actorId:id,device:clean(event.device,100),label:clean(event.label,300)||'Edited tracker',operation:clean(event.operation,32)||'change',target:clean(event.target,100),before:parse(s.state_json),after,version};
   const encoded=JSON.stringify(accepted);
   const results=await db.batch([
    db.prepare("INSERT OR IGNORE INTO events(id,store_id,version,actor_id,actor_email,actor_name,at,payload_json) SELECT ?,id,?,?,?,?,?,? FROM stores WHERE id=? AND version=? AND archived=0 AND (type='shared' OR owner_id=?) AND EXISTS(SELECT 1 FROM memberships WHERE store_id=stores.id AND user_id=? AND (role='admin' OR (?!='policy' AND role='editor')))").bind(event.id,version,id,email,accepted.actor,accepted.at,encoded,sid,s.version,id,id,sub),
    db.prepare('UPDATE stores SET state_json=?,version=?,updated_at=? WHERE id=? AND version=? AND EXISTS(SELECT 1 FROM events WHERE id=? AND store_id=? AND actor_id=? AND payload_json=?)').bind(JSON.stringify(after),version,accepted.at,sid,s.version,event.id,sid,id,encoded)
   ]);
   if(!results[0]?.meta?.changes){
    const raced=await db.prepare('SELECT store_id,version,payload_json FROM events WHERE id=?').bind(event.id).first();
    if(raced){const replay=retryResult(raced,sid,user,intent);return json({version:replay.version,duplicate:true,...(!dedicated&&!hasScopedAccess(source)&&!hasScopedAccess(replay.event.before)&&!hasScopedAccess(replay.event.after)?{event:publicEvent(replay.event)}:{})});}
    return json({error:'Store changed concurrently. Reload and merge.',version:(await access(db,sid,user)).version},409);
   }
   return json({version,...(!dedicated&&!hasScopedAccess(source)&&!hasScopedAccess(after)?{event:publicEvent(accepted)}:{}),...(requestResult?{requestId:requestResult.id}:{})});
  }
  if(sub==='settings'&&method==='POST'){
   const s=await access(db,sid,user,'admin'),b=await body(request);
   const currentType=s.is_open?'open':s.type;
   const name='name'in b?clean(b.name,160):s.name,type=b.type||currentType,archived='archived'in b?!!b.archived:!!s.archived;
   if(!name||!['private','shared','open'].includes(type))fail(400,'Invalid settings');
   if(type!==currentType&&(type==='open'||currentType==='open'))fail(409,'Open stores cannot be converted. Create a separate store to avoid exposing old history.');
   if(type!==currentType&&(await db.prepare('SELECT key FROM default_stores WHERE store_id=?').bind(sid).first()))fail(409,'Default personal and shared stores retain their original type. Create another store for a different audience.');
   if(type!==currentType&&s.owner_id!==id)fail(403,'Only the owner can change store type');
   await adminMutation(db,sid,user,'store.settings',{before:{name:s.name,type:s.type,archived:!!s.archived},after:{name,type,archived}},receipt=>[db.prepare('UPDATE stores SET name=?,type=?,archived=?,updated_at=? WHERE id=? AND EXISTS(SELECT 1 FROM access_events WHERE id=?)').bind(name,type==='open'?'shared':type,+archived,now(),sid,receipt)]);
   return json({store:storeData(await access(db,sid,user))});
  }
  if(sub==='access'&&method==='GET'){
   await access(db,sid,user,'admin');
   const [members,invites,history]=await Promise.all([
    db.prepare('SELECT user_id AS userId,email,role,created_at AS createdAt FROM memberships WHERE store_id=?').bind(sid).all(),
    db.prepare('SELECT id,email,role,status,created_at AS createdAt,responded_at AS respondedAt FROM invitations WHERE store_id=? ORDER BY created_at').bind(sid).all(),
    db.prepare('SELECT id,actor_id AS actorId,at,action,details_json AS details FROM access_events WHERE store_id=? ORDER BY at').bind(sid).all()
   ]);
   return json({members:members.results,invitations:invites.results,history:history.results.map(x=>({...x,details:parse(x.details)}))});
  }
  if(sub==='members'&&path[4]&&method==='POST'){
   const s=await access(db,sid,user,'admin'),b=await body(request),uid=path[4];
   if(uid===s.owner_id)fail(403,'The store owner always retains admin access');
   if(!['reader','editor','admin','remove'].includes(b.role))fail(400,'Invalid role');
   await adminMutation(db,sid,user,'member.role',{userId:uid,role:b.role},receipt=>[b.role==='remove'?db.prepare('DELETE FROM memberships WHERE store_id=? AND user_id=? AND EXISTS(SELECT 1 FROM access_events WHERE id=?)').bind(sid,uid,receipt):db.prepare('UPDATE memberships SET role=? WHERE store_id=? AND user_id=? AND EXISTS(SELECT 1 FROM access_events WHERE id=?)').bind(b.role,sid,uid,receipt)]);
   return json({ok:true});
  }
  if(sub==='invites'&&method==='POST'){
   const s=await access(db,sid,user,'admin',false);
   if(path[4]&&path[5]==='revoke'){
    await adminMutation(db,sid,user,'invite.revoked',{inviteId:path[4]},receipt=>[db.prepare("UPDATE invitations SET status='revoked',responded_at=? WHERE id=? AND store_id=? AND status='pending' AND EXISTS(SELECT 1 FROM access_events WHERE id=?)").bind(now(),path[4],sid,receipt)],{activeShared:true});
    return json({ok:true});
   }
   if(path.length!==4)fail(404,'Route not found');
   if(s.type!=='shared')fail(409,'Change store type to shared before inviting');
   const b=await body(request),email=clean(b.email,320).toLowerCase(),role=roleValue(b.role||'reader');
   if(!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email))fail(400,'Valid email required');
   const existing=await db.prepare("SELECT id,role FROM invitations WHERE store_id=? AND email=? AND status='pending'").bind(sid,email).first();
   if(existing)return json({invitation:{id:existing.id,email,role:existing.role,status:'pending'},sent:false,note:'Invitation already pending. No email was sent.'});
   const iid=crypto.randomUUID(),at=now();
   await adminMutation(db,sid,user,'invite.created',{inviteId:iid,email,role},receipt=>[db.prepare('INSERT INTO invitations(id,store_id,email,role,created_by,created_at) SELECT ?,?,?,?,?,? WHERE EXISTS(SELECT 1 FROM access_events WHERE id=?)').bind(iid,sid,email,role,id,at,receipt)],{activeShared:true});
   return json({invitation:{id:iid,email,role,status:'pending'},sent:false,note:'Invitation saved, not emailed. Invitees must be allowed through the Site sharing policy before accepting.'},201);
  }
  fail(404,'Route not found');
 }catch(e){return json({error:e.status?e.message:'Database request failed. Your previous saved version remains available.'},e.status||500)}
}
