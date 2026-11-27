// BA-W01 L7 (contract §5): the enforcement seams for history, media, exports, summaries and background
// jobs. Every surface asks the §4 evaluator in access.mjs, through projectSnapshot or compileAccess,
// with the principal that started the work. None filters with logic of its own, and each answers a
// denied entity exactly as it answers one that never existed. The annex
// doc/2026-11-13T07-28-00Z_design_bedside-and-away/ba-w01-l7-enforcement-annex.md traces each one to §5.
import {compileAccess,projectSnapshot,projectEvents} from './access.mjs';

const copy=x=>structuredClone(x);
const problem=(status,message)=>Object.assign(new Error(message),{status});

/** The one denial for a media fetch or a job target: the same as an entity that was never there. */
export const NOT_FOUND=Object.freeze({allowed:false,status:404,error:'Not found'});

/** Entity kinds a surface can name, and the snapshot field that holds them. */
export const ENTITY_KINDS=Object.freeze({item:'items',tag:'tags',note:'notes',inventory:'inventory'});

/** History reader (§5): denied entities are omitted, never redacted to a placeholder. */
export function historyView(events,state,context){return projectEvents(events,state,context)}

/**
 * Media fetch route (§5): the row the media belongs to, as the principal may see it, or null. Absent
 * and forbidden both give null, so the route answers {@link NOT_FOUND} for both.
 */
export function mediaLookup(state,context,{kind,id}={}){
 if(!Object.hasOwn(ENTITY_KINDS,kind)||typeof id!=='string'||!id)return null;
 const row=(projectSnapshot(state,context)[ENTITY_KINDS[kind]]||[]).find(r=>r?.id===id);
 return row?copy(row):null;
}

export const EXPORT_SCHEMA='bedaway.store-export';
/**
 * Export builder (§5): only the rows the principal may read, with the store revision they came from
 * stated, so an export can be traced to one snapshot.
 */
export function buildExport(state,context,{storeId,sourceRevision}){
 if(typeof storeId!=='string'||!storeId||!Number.isSafeInteger(sourceRevision)||sourceRevision<0)throw problem(500,'Export needs a store and its revision');
 return {schema:EXPORT_SCHEMA,version:1,storeId,sourceRevision,state:projectSnapshot(state,context)};
}

/**
 * Report pipeline input (§5): summaries and aggregates are computed over this and nothing else, so a
 * total never counts a row the principal cannot read.
 */
export function summaryInput(state,context){return projectSnapshot(state,context)}

/**
 * Background-job envelope (§5): a job carries the principal that started it and the store role that
 * principal held, and is authorized as that principal only. It never gains broader visibility.
 */
export function jobEnvelope({principal,role,storeId}){
 if(!principal?.id||typeof role!=='string'||!role||typeof storeId!=='string'||!storeId)throw problem(403,'A job needs the principal that started it');
 return Object.freeze({storeId,role,initiator:Object.freeze({id:principal.id})});
}

/**
 * Authorize every target of a job as its initiator. A target the initiator may not act on, or cannot
 * see, gives {@link NOT_FOUND}, the same answer as a target that does not exist.
 */
export function authorizeJobTargets(state,envelope,targets,action='write'){
 if(!envelope?.initiator?.id||!Array.isArray(targets)||!targets.length)return {...NOT_FOUND};
 const context={principal:envelope.initiator,role:envelope.role};
 const decide=compileAccess({...context,access:state?.access});
 if(!decide('*',action).allowed)return {...NOT_FOUND};
 let view=null;
 for(const t of targets){
  if(typeof t?.kind!=='string'||!t.kind||typeof t.id!=='string'||!t.id)return {...NOT_FOUND};
  if(!decide(t.kind+':'+t.id,action).allowed)return {...NOT_FOUND};
  if(Object.hasOwn(ENTITY_KINDS,t.kind)){
   view??=projectSnapshot(state,context);
   if(!(view[ENTITY_KINDS[t.kind]]||[]).some(r=>r?.id===t.id))return {...NOT_FOUND};
  }
 }
 return {allowed:true};
}

/**
 * The `authorize` resolver the steering outbox needs, routed through the evaluator. `load` reads one
 * store snapshot for the job: {state, role, version, request, targetsFresh}, or null when the actor is
 * no longer a member. The outbox then uses `storeVersion` in every write predicate.
 */
export function jobAuthorizer(load){
 if(typeof load!=='function')throw problem(503,'Store loader required');
 return async({storeId,requestId,actorId})=>{
  const s=await load({storeId,requestId,actorId});
  if(!s)return {allowed:false};
  const envelope=jobEnvelope({principal:{id:actorId},role:s.role,storeId});
  const targets=(s.request?.targets||[]).map(t=>({kind:t?.kind,id:t?.id}));
  return {allowed:authorizeJobTargets(s.state,envelope,targets).allowed,storeVersion:s.version,request:s.request,targetsFresh:s.targetsFresh===true};
 };
}
