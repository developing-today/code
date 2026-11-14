/* Admin-only object-policy editor. All authority and validation remain on the server. */
(()=>{
 'use strict';
 const object=x=>x&&typeof x==='object'&&!Array.isArray(x);
 const safeKey=k=>typeof k==='string'&&k.trim()&&k.length<=200&&!['__proto__','prototype','constructor'].includes(k)&&!/[\p{Cc}\p{Cf}]/u.test(k);
 const clone=x=>JSON.parse(JSON.stringify(x));
 function parseDraft(text){let d;try{d=JSON.parse(text)}catch{throw Error('Enter valid JSON. No changes were saved.')}
  if(!object(d)||d.schemaVersion!==1||!object(d.groups)||!object(d.policies))throw Error('Expected schemaVersion 1, groups object, and policies object.');
  for(const [id,m]of Object.entries(d.groups))if(!safeKey(id)||!Array.isArray(m)||m.some(x=>typeof x!=='string'||!x))throw Error('Each group needs a name and account ID list.');
  for(const [id,p]of Object.entries(d.policies))if(!safeKey(id)||!object(p)||!Array.isArray(p.rules))throw Error('Each policy needs a resource key and rules array.');
  return d;
 }
 function putGroup(d,key,members){if(!safeKey(key))throw Error('Use a nonempty group ID without control characters.');d.groups[key]=[...new Set(members.map(x=>x.trim()).filter(Boolean))];return d}
 function removeGroup(d,key){for(const p of Object.values(d.policies))if(p.rules.some(r=>r.subject?.type==='group'&&r.subject.id===key))throw Error('Remove this group from policy rules first.');delete d.groups[key];return d}
 function putPolicy(d,key,policy){if(!safeKey(key)||!object(policy)||!Array.isArray(policy.rules))throw Error('Policy needs a resource key and a rules array.');d.policies[key]=clone(policy);return d}
 function removePolicy(d,key){for(const p of Object.values(d.policies))if(p.inherit?.includes(key))throw Error('Remove inherited references to this policy first.');delete d.policies[key];return d}
 const canManage=c=>!!c?.storeId&&c.role==='admin'&&!c.archived;
 function node(tag,text,attrs={}){const n=document.createElement(tag);if(text!==undefined)n.textContent=text;for(const [k,v]of Object.entries(attrs))n.setAttribute(k,v);return n}
 function labeled(title,input){const l=node('label',title);l.append(input);return l}
 function button(text,click){const b=node('button',text,{type:'button'});b.addEventListener('click',click);return b}
 async function open({getContext,request,reload}){
  const first=getContext();if(!canManage(first))throw Error('An active store with administrator access is required.');
  const storeId=first.storeId,base='stores/'+encodeURIComponent(storeId),dialog=node('dialog',undefined,{'aria-label':'Store access policies'});
  dialog.style.cssText='max-width:min(900px,95vw);width:95vw;max-height:90vh;overflow:auto';
  const title=node('h2','Object access and groups'),status=node('p','Checking policy endpoint…',{'role':'status'}),body=node('div'),close=button('Close',()=>dialog.close());
  dialog.append(title,status,close,body);document.body.append(dialog);dialog.addEventListener('close',()=>dialog.remove(),{once:true});dialog.showModal();
  let busy=false,loaded,review=null,raw;
  const current=()=>{const c=getContext();if(!canManage(c)||c.storeId!==storeId)throw Error('Store or administrator access changed. Close and reopen.');return c};
  const run=async fn=>{if(busy)return;busy=true;try{current();await fn()}catch(e){status.textContent=e.message||'Request failed.'}finally{busy=false}};
  async function load(){const out=await request(base+'/policy');if(!dialog.isConnected)return;current();
   if(!Number.isSafeInteger(out.version)||!Array.isArray(out.members))throw Error('Policy endpoint unavailable or incompatible. No policy changes are enabled.');
   loaded=out;review=null;render();status.textContent='Live server policies loaded · store revision '+out.version+'. Changes below are drafts until confirmed.';
  }
  function draft(){return parseDraft(raw.value)}
  function set(d){raw.value=JSON.stringify(d,null,2);review=null;status.textContent='Unsaved draft. Review changes before applying.'}
  function mutate(fn){try{const d=draft();fn(d);set(d)}catch(e){status.textContent=e.message}}
  function render(){body.replaceChildren();
   const warning=node('p','Policies affect content visibility, search and history. Scoped stores currently disable ordinary snapshot editing. Administrators cannot save a policy that locks themselves or the owner out. Lower numbers are stronger; deny wins ties. Browser identifiers never grant access.');body.append(warning);
   const members=node('section');members.append(node('h3','Store members'));
   for(const m of loaded.members){const row=node('div');row.style.cssText='display:flex;gap:.5rem;flex-wrap:wrap;margin:.5rem 0';row.append(node('span',(m.email||m.userId)+' · '+m.userId));const role=node('select',undefined,{'aria-label':'Role for '+m.userId});for(const v of ['reader','editor','admin','remove']){const o=node('option',v,{value:v});role.append(o)}role.value=m.role;
    row.append(role,button('Update role',()=>run(async()=>{if(!window.confirm('Change '+m.userId+' from '+m.role+' to '+role.value+'? This changes their store access.'))return;await request(base+'/members/'+encodeURIComponent(m.userId),{role:role.value});await reload(storeId);if(!canManage(getContext())){dialog.close();return}await load()})));members.append(row);
   }
   const inviteEmail=node('input',undefined,{type:'email',placeholder:'Invite email','aria-label':'Invitation email'}),inviteRole=node('select',undefined,{'aria-label':'Invitation role'});for(const v of ['reader','editor','admin'])inviteRole.append(node('option',v,{value:v}));
   members.append(inviteEmail,inviteRole,button('Save invitation',()=>run(async()=>{if(!inviteEmail.checkValidity()||!inviteEmail.value.trim())throw Error('Enter a valid invitation email.');if(!window.confirm('Save an invitation for '+inviteEmail.value+' as '+inviteRole.value+'? No email or Tailscale invitation is sent.'))return;await request(base+'/invites',{email:inviteEmail.value,role:inviteRole.value});status.textContent='Invitation saved. This does not grant access through the private site gateway.';})));body.append(members);
   raw=node('textarea',undefined,{'aria-label':'Complete access policy JSON',rows:'18',spellcheck:'false'});raw.style.cssText='width:100%;font-family:monospace';raw.value=JSON.stringify(loaded.access||{schemaVersion:1,groups:{},policies:{}},null,2);raw.addEventListener('input',()=>{review=null;status.textContent='Unsaved draft. Review changes before applying.'});
   const groups=node('section');groups.append(node('h3','Groups'));const gid=node('input',undefined,{placeholder:'Group ID','aria-label':'Group ID'}),ids=node('textarea',undefined,{placeholder:'Account IDs, one per line','aria-label':'Group account IDs',rows:'3'});
   const list=node('select',undefined,{'aria-label':'Existing group'});list.append(node('option','Choose group',{value:''}));for(const id of Object.keys(loaded.access?.groups||{}))list.append(node('option',id,{value:id}));list.addEventListener('change',()=>{gid.value=list.value;try{ids.value=(draft().groups[list.value]||[]).join('\n')}catch(e){status.textContent=e.message}});
   groups.append(list,labeled('Group ID',gid),labeled('Members (account IDs)',ids),button('Add / update group draft',()=>mutate(d=>putGroup(d,gid.value,ids.value.split('\n')))),button('Remove group draft',()=>mutate(d=>removeGroup(d,gid.value))));body.append(groups);
   const policy=node('section');policy.append(node('h3','Resource policies'));const key=node('input',undefined,{placeholder:'item:ID, tag:ID or *','aria-label':'Resource policy key'}),rules=node('textarea',undefined,{'aria-label':'Resource policy JSON',rows:'8',spellcheck:'false'});rules.style.width='100%';rules.value=JSON.stringify({rules:[]},null,2);
   const resources=node('select',undefined,{'aria-label':'Existing policy'});resources.append(node('option','Choose policy',{value:''}));for(const id of Object.keys(loaded.access?.policies||{}))resources.append(node('option',id,{value:id}));resources.addEventListener('change',()=>{key.value=resources.value;try{rules.value=JSON.stringify(draft().policies[resources.value]||{rules:[]},null,2)}catch(e){status.textContent=e.message}});
   policy.append(resources,labeled('Resource key',key),labeled('Policy JSON',rules),button('Add / update policy draft',()=>mutate(d=>{let p;try{p=JSON.parse(rules.value)}catch{throw Error('Resource policy JSON is invalid.')}putPolicy(d,key.value,p)})),button('Remove policy draft',()=>mutate(d=>removePolicy(d,key.value))));
   policy.append(node('p','Example read restriction: {"default":"deny","rules":[{"effect":"allow","subject":{"type":"user","id":"ACCOUNT_ID"},"actions":["read","write","admin"]}]}. Include the owner and your administrator account; the server rejects self-lockout.'));body.append(policy,node('h3','Complete proposed policy'),raw);
   const reviewBox=node('div');body.append(button('Review policy changes',()=>{try{current();const proposed=draft();review={proposed,serialized:JSON.stringify(proposed),commandId:crypto.randomUUID()};reviewBox.replaceChildren(node('h3','Confirm visibility changes'),node('p','This will replace the store’s policy and group configuration at revision '+loaded.version+'. Removing a restriction can make records visible to more existing store members. Restricting visibility can hide history and disable snapshot editing.'));
    const preview=node('pre',JSON.stringify(proposed,null,2));preview.style.cssText='white-space:pre-wrap;overflow-wrap:anywhere';const consent=node('input',undefined,{type:'checkbox'});reviewBox.append(preview,labeled('I reviewed the access changes and want to apply this policy.',consent),button('Apply reviewed policy',()=>run(async()=>{if(!consent.checked)throw Error('Confirm that you reviewed the access changes.');if(!review||JSON.stringify(draft())!==review.serialized)throw Error('Draft changed. Review it again.');await request(base+'/policy',{expectedVersion:loaded.version,commandId:review.commandId,access:review.proposed});await load();await reload(storeId);status.textContent='Policy saved and reloaded from the server.';})));}catch(e){status.textContent=e.message}}),button('Reload saved policies',()=>run(load)),reviewBox);
  }
  await run(load);
 }
 window.BedawayAccessUI={open,parseDraft,putGroup,removeGroup,putPolicy,removePolicy,canManage};
})();
function accessPolicyButton(){return active?.role==='admin'&&!active.archived?button('access-policy','Object policies & groups'):''}
function accessPolicyOpen(){return window.BedawayAccessUI.open({getContext:()=>({storeId:active?.id,role:active?.role,archived:active?.archived}),request:call,reload:openStore}).catch(err=>notice(err.message))}
if(typeof document!=='undefined')document.addEventListener('click',ev=>{if(ev.target.closest('button')?.dataset.act==='access-policy')accessPolicyOpen()});
