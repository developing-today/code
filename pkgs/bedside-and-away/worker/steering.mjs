// Pure, server-owned state machine. Persist returned state and audit atomically using store CAS.
const clone=x=>structuredClone(x);
const stable=x=>JSON.stringify(x,(k,v)=>v&&typeof v==='object'&&!Array.isArray(v)?Object.fromEntries(Object.entries(v).sort(([a],[b])=>a.localeCompare(b))):v);
const error=(status,message)=>{throw Object.assign(new Error(message),{status})};
const id=x=>typeof x==='string'&&/^[A-Za-z0-9_-]{1,100}$/.test(x);
const prose=(s,max=12000)=>typeof s==='string'?s.normalize('NFC').replace(/[\p{Cc}\p{Cf}]/gu,' ').trim().slice(0,max):'';
const collection=(s,kind)=>kind==='item'?s.items:kind==='category'?s.tags:null;
const key=t=>t.kind+':'+t.id;
const find=(s,t)=>collection(s,t.kind)?.find(x=>x.id===t.id);
const terminal=new Set(['applied','cancelled','completed']);
export function assertSteeringUnchanged(before,after){
 for(const field of ['steering','steeringReceipts','steeringAudit'])if(stable(before[field]??[])!==stable(after[field]??[]))error(400,'Use the dedicated steering route to change requests');
}
export function orderedByPriority(items){return [...items].sort((a,b)=>(Number.isFinite(b.priority)?b.priority:0)-(Number.isFinite(a.priority)?a.priority:0));}
function changes(raw,kind){
 if(!raw||typeof raw!=='object'||Array.isArray(raw)||!Object.keys(raw).length)error(400,'Explicit changes required');
 const out={};for(const [k,v] of Object.entries(raw)){
  if(!['name','description','priority',...(kind==='item'?['state']:[])].includes(k))error(400,'Unsupported change field');
  if(k==='priority'){if(!Number.isInteger(v)||v<0||v>100)error(400,'Priority must be 0–100');out[k]=v;}
  else {if(typeof v!=='string')error(400,'Text field required');out[k]=prose(v,k==='name'?300:12000);if(k==='name'&&!out[k])error(400,'Name required');if(k==='state'&&!['Considering','Shortlisted','Bought','Different direction','Passed','Need replacement'].includes(v))error(400,'Invalid decision state');}
 }return out;
}
export function transitionSteering({state,storeId,storeVersion,actor,role,command,now=new Date().toISOString(),canAccessTarget=()=>true}){
 if(!['editor','admin'].includes(role)||!actor?.id)error(403,'Editor access required');
 if(!id(storeId)||!command||!id(command.commandId)||!id(command.requestId))error(400,'Valid command and request IDs required');
 if(!Number.isInteger(storeVersion)||storeVersion<0)error(400,'Store version required');
 const signature=stable({actorId:actor.id,command});
 const prior=(state.steeringReceipts??[]).find(x=>x.commandId===command.commandId);
 if(prior){if(prior.signature!==signature)error(409,'Command ID collision');const request=(state.steering??[]).find(x=>x.id===prior.requestId);for(const t of request?.targets??[]){const current=find(state,t);if(!current||!canAccessTarget(t,current,'edit'))error(404,'Target not available');}return {state,request:clone(request),audit:clone(prior.audit),duplicate:true};}
 const next=clone(state);next.steering??=[];next.steeringReceipts??=[];next.steeringAudit??=[];
 let r=next.steering.find(x=>x.id===command.requestId);
 const capture=targets=>{
  if(!Array.isArray(targets)||targets.length>100)error(400,'At most 100 targets');
  const seen=new Set();return targets.map(t=>{
   if(!t||!id(t.id)||!['item','category'].includes(t.kind)||seen.has(key(t)))error(400,'Invalid or duplicate target');seen.add(key(t));
   const current=find(next,t);if(!current||!canAccessTarget(t,current,'edit'))error(404,'Target not available');return {kind:t.kind,id:t.id,before:clone(current)};
  });
 };
 if(command.action==='create'){
  if(r)error(409,'Request ID already exists');const text=prose(command.text);if(!text)error(400,'Request text required');
  const targets=capture(command.targets??[]);
  r={id:command.requestId,schema:'bedaway.steering',schemaVersion:2,revision:1,storeId,baseVersion:storeVersion,scope:targets.length?'targets':'store',targets,request:text,rationale:prose(command.rationale),status:'pending',actorId:actor.id,author:prose(actor.name??actor.id,100),at:now};next.steering.push(r);
 }else{
  if(!r)error(404,'Request not found');if(r.schemaVersion!==2)error(409,'Legacy request is read-only; create a new request');
  if(r.revision!==command.expectedRevision)error(409,'Request revision changed');if(terminal.has(r.status))error(409,'Request already resolved');
  for(const t of r.targets){const current=find(next,t);if(!current||!canAccessTarget(t,current,'edit'))error(404,'Target not available');}
  const fresh=()=>{for(const t of r.targets)if(stable(find(next,t))!==stable(t.before))error(409,'Target changed; refresh baseline and propose again');};
  if(command.action==='propose'){
   fresh();if(!Array.isArray(command.patches)||!command.patches.length||command.patches.length>100)error(400,'Proposal patches required');const seen=new Set();
   r.proposal={patches:command.patches.map(p=>{if(!p||!r.targets.some(t=>key(t)===key(p))||seen.has(key(p)))error(400,'Patch is outside request target scope');seen.add(key(p));return {kind:p.kind,id:p.id,changes:changes(p.changes,p.kind)};}),by:actor.id,at:now};r.status='proposed';
  }else if(command.action==='apply'){
   if(r.status!=='proposed'||!r.proposal)error(409,'Review a proposal before applying');fresh();
   for(const p of r.proposal.patches)Object.assign(find(next,p),changes(p.changes,p.kind));r.status='applied';r.applied={by:actor.id,at:now,patches:clone(r.proposal.patches)};
  }else if(command.action==='refresh'){
   r.targets=capture(r.targets.map(({kind,id})=>({kind,id})));r.baseVersion=storeVersion;r.proposal=null;r.status='pending';
  }else if(command.action==='cancel'||command.action==='complete'){
   if(command.action==='complete'&&!prose(command.resolution))error(400,'Resolution required');r.status=command.action==='cancel'?'cancelled':'completed';
  }else error(400,'Unsupported steering action');
  r.revision++;r.resolution=prose(command.resolution);r.reviewedAt=now;
 }
 const audit={id:command.commandId,requestId:r.id,action:command.action,actorId:actor.id,at:now,revision:r.revision,storeVersion,request:clone(r)};
 next.steeringAudit.push(audit);next.steeringReceipts.push({commandId:command.commandId,requestId:r.id,signature,audit:clone(audit)});
 return {state:next,request:clone(r),audit,duplicate:false};
}
const bytes=s=>new TextEncoder().encode(s);
async function signingKey(secret){return crypto.subtle.importKey('raw',bytes(secret),{name:'HMAC',hash:'SHA-256'},false,['sign','verify']);}
function configCheck(config){if(config?.enabled!==true)error(503,'Archer delivery is disabled');let url;try{url=new URL(config.endpoint)}catch{error(503,'Exact HTTPS endpoint required')};if(url.protocol!=='https:'||url.username||url.password||url.hash||!config.recipient||typeof config.secret!=='string'||config.secret.length<32)error(503,'Scoped endpoint and signing secret required');}
export async function makeSteeringEnvelope(request,config,{now=Math.floor(Date.now()/1000)}={}){
 configCheck(config);if(request.schemaVersion!==2)error(400,'Only versioned requests may be exported');
 const payload={schema:'bedaway.archer-job',version:1,jobId:request.id+':'+request.revision,storeId:request.storeId,recipient:config.recipient,issuedAt:now,expiresAt:now+300,request:clone(request),authority:'proposal-only; never change access, credentials, or deploy'};
 const signature=Array.from(new Uint8Array(await crypto.subtle.sign('HMAC',await signingKey(config.secret),bytes(stable(payload))))).map(x=>x.toString(16).padStart(2,'0')).join('');return {payload,signature};
}
export async function verifySteeringEnvelope(envelope,config,{now=Math.floor(Date.now()/1000)}={}){
 try{configCheck(config);const p=envelope.payload;if(p.recipient!==config.recipient||p.schema!=='bedaway.archer-job'||p.version!==1||!Number.isFinite(p.issuedAt)||!Number.isFinite(p.expiresAt)||p.issuedAt>now+30||p.expiresAt<now||p.expiresAt-p.issuedAt!==300||!/^[0-9a-f]{64}$/.test(envelope.signature))return false;return crypto.subtle.verify('HMAC',await signingKey(config.secret),Uint8Array.from(envelope.signature.match(/../g),x=>parseInt(x,16)),bytes(stable(p)));}catch{return false;}
}
