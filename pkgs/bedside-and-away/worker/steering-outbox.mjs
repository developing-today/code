import {makeSteeringEnvelope} from './steering.mjs';
const fail=(status,message)=>{throw Object.assign(Error(message),{status})};
const stable=x=>JSON.stringify(x,(k,v)=>v&&typeof v==='object'&&!Array.isArray(v)?Object.fromEntries(Object.entries(v).sort(([a],[b])=>a.localeCompare(b))):v);
const encode=s=>new TextEncoder().encode(s);
async function key(secret){return crypto.subtle.importKey('raw',encode(secret),{name:'HMAC',hash:'SHA-256'},false,['sign','verify']);}
export async function signResult(payload,secret){if(typeof secret!=='string'||secret.length<32)fail(503,'Signing key required');return {payload,signature:Array.from(new Uint8Array(await crypto.subtle.sign('HMAC',await key(secret),encode(stable(payload))))).map(v=>v.toString(16).padStart(2,'0')).join('')};}
function gate(config){if(config?.enabled!==true||config.operatorApproved!==true)fail(503,'External steering disabled until operator approval');if(!config.allowedDestinations?.includes(config.endpoint))fail(503,'Destination is not explicitly allowlisted');let u;try{u=new URL(config.endpoint)}catch{fail(503,'HTTPS endpoint required')};if(u.protocol!=='https:'||u.username||u.password||u.hash||typeof config.secret!=='string'||config.secret.length<32||!config.recipient)fail(503,'Secure recipient configuration required');}
const safeId=x=>typeof x==='string'&&/^[A-Za-z0-9_:-]{1,256}$/.test(x);
function validateResult(p,job,now){
 if(p?.schema!=='bedaway.archer-result'||p.version!==1||!safeId(p.callbackId)||p.jobId!==job.id||p.recipient!==job.recipient||p.requestRevision!==job.request_revision)fail(400,'Invalid result identity');
 if(!Number.isFinite(p.issuedAt)||!Number.isFinite(p.expiresAt)||p.issuedAt>now+30||p.expiresAt<now||p.expiresAt-p.issuedAt!==300)fail(401,'Expired result');
 if(typeof p.summary!=='string'||p.summary.length>12000||!Array.isArray(p.patches)||p.patches.length>100)fail(400,'Invalid proposal result');
 const request=JSON.parse(job.request_json),seen=new Set();for(const patch of p.patches){const target=patch?.kind+':'+patch?.id;if(!request.targets.some(t=>t.kind+':'+t.id===target)||seen.has(target))fail(400,'Result outside target scope');seen.add(target);if(!patch.changes||typeof patch.changes!=='object'||Array.isArray(patch.changes)||!Object.keys(patch.changes).length)fail(400,'Invalid changes');for(const [field,value] of Object.entries(patch.changes)){if(!['name','description','priority',...(patch.kind==='item'?['state']:[])].includes(field))fail(400,'Forbidden result field');if(field==='priority'){if(!Number.isInteger(value)||value<0||value>100)fail(400,'Invalid priority');}else if(typeof value!=='string'||value.length>12000||(field==='name'&&!value.trim())||(field==='state'&&!['Considering','Shortlisted','Bought','Different direction','Passed','Need replacement'].includes(value)))fail(400,'Invalid result value');}}
 if(stable(p).length>100000)fail(413,'Result too large');
}
// authorize must obtain current membership/object edit checks and request in one store snapshot.
// Its returned version is used in every write predicate, closing authorization races.
export function createSteeringOutbox({db,config,authorize,clock=()=>Math.floor(Date.now()/1000)}){
 if(typeof authorize!=='function')fail(503,'Current authorization resolver required');
 const get=async id=>db.prepare('SELECT * FROM steering_outbox WHERE id=?').bind(id).first();
 const predicate="EXISTS(SELECT 1 FROM stores s JOIN memberships m ON m.store_id=s.id WHERE s.id=? AND s.version=? AND s.archived=0 AND m.user_id=? AND m.role IN ('editor','admin'))";
 async function check(job){const a=await authorize({storeId:job.store_id,requestId:job.request_id,actorId:job.actor_id});if(!a?.allowed||!Number.isInteger(a.storeVersion))fail(403,'Request authority revoked');if(a.targetsFresh!==true)fail(409,'Request targets are stale');if(!a.request||a.request.revision!==job.request_revision||!['pending','proposed'].includes(a.request.status)||stable(a.request)!==job.request_json)fail(409,'Request is stale');return a;}
 return {get,
 async enqueue({storeId,requestId,revision,actorId}){
  gate(config);const a=await authorize({storeId,requestId,actorId});if(!a?.allowed||!Number.isInteger(a.storeVersion))fail(403,'Request authority revoked');if(a.targetsFresh!==true)fail(409,'Request targets are stale');const r=a.request;if(!r||r.id!==requestId||r.storeId!==storeId||r.schemaVersion!==2||r.revision!==revision||!['pending','proposed'].includes(r.status))fail(409,'Request is stale');
  const id=storeId+':'+requestId+':'+revision,at=clock(),raw=stable(r);const prior=await get(id);if(prior){if(prior.request_json!==raw||prior.actor_id!==actorId||prior.destination!==config.endpoint||prior.recipient!==config.recipient)fail(409,'Job ID collision');return {...prior,duplicate:true};}
  const result=await db.prepare(`INSERT OR IGNORE INTO steering_outbox(id,store_id,request_id,request_revision,actor_id,request_json,destination,recipient,created_at,updated_at) SELECT ?,?,?,?,?,?,?,?,?,? WHERE ${predicate}`).bind(id,storeId,requestId,revision,actorId,raw,config.endpoint,config.recipient,at,at,storeId,a.storeVersion,actorId).run();
  if(!result.meta.changes){const raced=await get(id);if(raced&&raced.request_json===raw&&raced.actor_id===actorId&&raced.destination===config.endpoint&&raced.recipient===config.recipient)return {...raced,duplicate:true};fail(409,'Store changed before enqueue');}return {...await get(id),duplicate:false};
 },
 async dispatch(id,fetcher){
  gate(config);if(typeof fetcher!=='function')fail(503,'Explicit network adapter required');const job=await get(id);if(!job)fail(404,'Job not found');if(job.destination!==config.endpoint||job.recipient!==config.recipient)fail(403,'Job destination no longer approved');if(job.state!=='pending')fail(409,'Job already dispatched or outcome unknown');const a=await check(job),lease=crypto.randomUUID(),at=clock();
  const envelope=await makeSteeringEnvelope(JSON.parse(job.request_json),config,{now:at});envelope.payload.jobId=job.id;
  // Re-sign because durable IDs include store scope; no signature may cover a different payload.
  const signed=await signResult(envelope.payload,config.secret);
  const claimed=await db.prepare(`UPDATE steering_outbox SET state='dispatching',lease_token=?,updated_at=? WHERE id=? AND state='pending' AND ${predicate}`).bind(lease,at,id,job.store_id,a.storeVersion,job.actor_id).run();if(!claimed.meta.changes)fail(409,'Job claimed or authorization changed');
  try{const response=await fetcher(job.destination,{method:'POST',redirect:'manual',headers:{'content-type':'application/json','idempotency-key':id},body:JSON.stringify(signed),signal:AbortSignal.timeout(15000)});if(!response.ok)throw Error('Non-successful delivery');await db.prepare("UPDATE steering_outbox SET state='sent',updated_at=? WHERE id=? AND state='dispatching' AND lease_token=?").bind(clock(),id,lease).run();return {id,state:(await get(id)).state};}
  catch{await db.prepare("UPDATE steering_outbox SET state='uncertain',updated_at=? WHERE id=? AND state='dispatching' AND lease_token=?").bind(clock(),id,lease).run();fail(503,'Delivery outcome unknown; reconcile before any resend');}
 },
 async receive(envelope){
  gate(config);if(!envelope||!safeId(envelope.payload?.jobId)||!/^[0-9a-f]{64}$/.test(envelope.signature??''))fail(401,'Invalid result signature');
  if(!await crypto.subtle.verify('HMAC',await key(config.secret),Uint8Array.from(envelope.signature.match(/../g),x=>parseInt(x,16)),encode(stable(envelope.payload))))fail(401,'Invalid result signature');
  const p=envelope.payload,job=await get(p.jobId);if(!job)fail(404,'Job not found');if(job.destination!==config.endpoint||job.recipient!==config.recipient)fail(403,'Job destination no longer approved');validateResult(p,job,clock());const a=await check(job),raw=stable(p);
  const existing=await db.prepare('SELECT * FROM steering_callbacks WHERE callback_id=? OR job_id=?').bind(p.callbackId,p.jobId).first();if(existing){if(existing.callback_id!==p.callbackId||existing.payload_json!==raw)fail(409,'Callback ID collision');return {duplicate:true,proposal:JSON.parse(existing.payload_json)};}
  if(!['dispatching','sent','uncertain'].includes(job.state))fail(409,'Job not awaiting result');
  const results=await db.batch([
   db.prepare(`INSERT OR IGNORE INTO steering_callbacks(callback_id,job_id,payload_json,received_at) SELECT ?,id,?,? FROM steering_outbox WHERE id=? AND state IN ('dispatching','sent','uncertain') AND ${predicate}`).bind(p.callbackId,raw,clock(),job.id,job.store_id,a.storeVersion,job.actor_id),
   db.prepare("UPDATE steering_outbox SET state='completed',updated_at=? WHERE id=? AND EXISTS(SELECT 1 FROM steering_callbacks c WHERE c.job_id=steering_outbox.id AND c.callback_id=? AND c.payload_json=?)").bind(clock(),job.id,p.callbackId,raw)
  ]);if(!results[0].meta.changes){const prior=await db.prepare('SELECT * FROM steering_callbacks WHERE job_id=?').bind(job.id).first();if(prior&&prior.callback_id===p.callbackId&&prior.payload_json===raw)return {duplicate:true,proposal:p};fail(409,'Concurrent callback or authority change');}
  return {duplicate:false,proposal:p}; // Stored only; never applies returned patches.
 }
 };
}
