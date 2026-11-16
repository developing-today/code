/** Server-only, versioned access policy. Input principal, role and access must be authoritative. */
const levels={reader:1,editor:2,admin:3},actions={read:1,write:2,admin:3};
const record=x=>x&&typeof x==='object'&&!Array.isArray(x);
const copy=x=>structuredClone(x);
const problem=message=>Object.assign(new Error(message),{status:403});
export function validateAccessConfig(access){
 if(access==null)return;
 if(!record(access)||access.schemaVersion!==1||!record(access.groups)||!record(access.policies))throw problem('Invalid access policy');
 const entries=Object.entries(access.policies);if(entries.length>12000||Object.keys(access.groups).length>1000)throw problem('Access policy too large');
 for(const members of Object.values(access.groups))if(!Array.isArray(members)||members.length>10000||members.some(x=>typeof x!=='string'||!x))throw problem('Invalid group');
 for(const [key,p]of entries){
  if(!key||!record(p)||p.default!==undefined&&!['allow','deny'].includes(p.default)||p.inherit!==undefined&&(!Array.isArray(p.inherit)||p.inherit.length>100)||!Array.isArray(p.rules)||p.rules.length>100)throw problem('Invalid policy');
  for(const parent of p.inherit||[])if(typeof parent!=='string'||!Object.hasOwn(access.policies,parent))throw problem('Missing policy parent');
  for(const r of p.rules){
   if(!record(r)||!['allow','deny'].includes(r.effect)||!record(r.subject)||!['everyone','user','group'].includes(r.subject.type)||r.subject.type!=='everyone'&&(typeof r.subject.id!=='string'||!r.subject.id)||!Array.isArray(r.actions)||!r.actions.length||r.actions.some(a=>!Object.hasOwn(actions,a))||r.priority!==undefined&&(!Number.isSafeInteger(r.priority)||r.priority<0||r.priority>1000000)||r.overrideDeny!==undefined&&typeof r.overrideDeny!=='boolean'||r.overrideDeny&&r.effect!=='allow')throw problem('Invalid policy rule');
   if(r.subject.type==='group'&&!Object.hasOwn(access.groups,r.subject.id))throw problem('Missing policy group');
  }
 }
 const done=new Set(),active=new Set();
 function visit(k,depth){if(depth>64||active.has(k))throw problem('Cyclic or deep policy inheritance');if(done.has(k))return;active.add(k);for(const p of access.policies[k].inherit||[])visit(p,depth+1);active.delete(k);done.add(k)}
 for(const [k]of entries)visit(k,0);
}
function decision({principal,role,entityId='*',access,action='read'},validated=false){
 const no=reason=>({allowed:false,reason});
 if(!principal?.id||!Object.hasOwn(actions,action)||!Object.hasOwn(levels,role)||levels[role]<actions[action])return no('store-role');
 if(!validated)try{validateAccessConfig(access)}catch{return no('invalid-policy')}
 if(!access)return {allowed:true,reason:'store-role'};
 const selected=new Set();function collect(k){if(selected.has(k)||!Object.hasOwn(access.policies,k))return;selected.add(k);for(const p of access.policies[k].inherit||[])collect(p)}
 collect('*');collect(entityId);
 const rules=[];let defaultDeny=false;
 for(const k of selected){const p=access.policies[k];defaultDeny ||= p.default==='deny';for(const r of p.rules){const s=r.subject;if(r.actions.includes(action)&&(s.type==='everyone'||s.type==='user'&&s.id===principal.id||s.type==='group'&&access.groups[s.id].includes(principal.id)))rules.push(r)}}
 const denies=rules.filter(r=>r.effect==='deny');
 const allows=rules.filter(r=>r.effect==='allow');
 if(denies.length){const strongest=Math.min(...denies.map(r=>r.priority??100));return allows.some(r=>r.overrideDeny&&(r.priority??100)<strongest)?{allowed:true,reason:'explicit-priority-override'}:no('inherited-deny')}
 if(defaultDeny&&!allows.length)return no('default-deny');
 return {allowed:true,reason:allows.length?'explicit-allow':'inherited-store-role'};
}
export function evaluateAccess(context){return decision(context)}
/** Validate once per immutable request snapshot, not once per record in a large store. */
export function compileAccess(context){
 const snapshot=copy(context);try{validateAccessConfig(snapshot.access)}catch{return ()=>({allowed:false,reason:'invalid-policy'})}
 const cache=new Map();return (entityId='*',action='read')=>{const key=JSON.stringify([entityId,action]);if(!cache.has(key))cache.set(key,decision({...snapshot,entityId,action},true));return {...cache.get(key)}};
}
const empty=()=>({items:[],tags:[],notes:[],inventory:[],people:[],prefs:{},rules:{},overrides:{},recs:{},research:[],budget:0,nickname:''});
function policyFor(state,context){return {...context,access:state.access}}
export function hasScopedAccess(state){return !!state.access&&Object.keys(state.access.policies||{}).length>0}
/** Fail closed for unknown aggregate fields in a scoped view: derive summaries after this projection. */
export function projectSnapshot(state,context){
 const ctx=policyFor(state,context),decide=compileAccess(ctx),can=(type,id)=>decide(`${type}:${id}`).allowed;
 if(!decide('*').allowed)return empty();
 const scoped=hasScopedAccess(state)||context.openVisitor;
 if(!scoped){const all=copy(state);if(context.role!=='admin')delete all.access;return all}
 const tags=(state.tags||[]).filter(t=>can('tag',t.id)&&(!context.openVisitor||!t.hidden&&!t.archived));
 const tagIds=new Set(tags.map(t=>t.id));
 const items=(state.items||[]).filter(i=>can('item',i.id)&&(!context.openVisitor||!i.hidden&&!i.archived));
 const ids=new Set(items.map(i=>i.id));
 const view=empty();
 view.tags=copy(tags);
 view.items=items.map(item=>{
  // Only documented scalar/canonical fields cross a scoped boundary. Embedded evidence is separate.
  const out={};for(const k of ['id','name','kind','owner','description','url','price','qty','store','hidden','archived','order','createdAt','updatedAt','revision','attributes'])if(Object.hasOwn(item,k))out[k]=copy(item[k]);
  out.tags=(item.tags||[]).filter(x=>tagIds.has(x));out.parent=ids.has(item.parent)?item.parent:'';
  for(const k of ['relatedIds','memberIds','variantIds'])if(Array.isArray(item[k]))out[k]=item[k].filter(x=>ids.has(x));
  return out;
 });
 // Known target contracts only. Unknown/global notes and computed aggregates may describe concealed records.
 const targets=r=>Array.isArray(r.targets)?r.targets:r.itemId?[r.itemId]:r.item?[r.item]:[];
 const safe=r=>{const ts=targets(r);return ts.length>0&&ts.every(t=>typeof t==='string'&&ids.has(t))};
 if(!context.openVisitor){view.notes=copy((state.notes||[]).filter(n=>safe(n)&&can('note',n.id)));view.inventory=copy((state.inventory||[]).filter(n=>safe(n)&&can('inventory',n.id)));}
 // Don't expose the existence, user/group lists or inheritance of inaccessible policies.
 return view;
}
/** Old snapshots/labels can disclose records no longer readable. Scoped history is intentionally withheld. */
export function projectEvents(events,currentState,context){
 if(hasScopedAccess(currentState)||context.openVisitor||!evaluateAccess({...policyFor(currentState,context),entityId:'*'}).allowed)return [];
 return copy(events.filter(e=>!hasScopedAccess(e.before||{})&&!hasScopedAccess(e.after||{})));
}
/** Snapshot writes are all-or-nothing. Scoped editors cannot safely rewrite state they cannot read. */
export function assertSnapshotMutation(before,after,context){
 if(!evaluateAccess({...policyFor(before,context),entityId:'*',action:'write'}).allowed)throw problem('Insufficient write access');
 validateAccessConfig(after.access);
 if(context.role!=='admin'){
  if(JSON.stringify(before.access??null)!==JSON.stringify(after.access??null))throw problem('Only administrators may edit access policies');
  if(hasScopedAccess(before)||hasScopedAccess(after))throw problem('Scoped stores require per-entity commands; snapshot editing is read-only');
 }
 if(context.role==='admin'&&hasScopedAccess(before)){
  // An admin denied data access may edit policy via a separate endpoint, never overwrite unseen data.
  for(const [field,type]of [['items','item'],['tags','tag'],['notes','note'],['inventory','inventory']])for(const r of before[field]||[])if(!evaluateAccess({...policyFor(before,context),entityId:`${type}:${r.id}`,action:'write'}).allowed&&JSON.stringify(r)!==JSON.stringify((after[field]||[]).find(x=>x.id===r.id)))throw problem('Cannot mutate denied entity');
 }
 return true;
}
