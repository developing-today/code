// Server-side compensating edits; never accept historical snapshots from the caller.
const copy=x=>x===undefined?undefined:structuredClone(x);
const stable=x=>JSON.stringify(x,(_,v)=>v&&typeof v==='object'&&!Array.isArray(v)?Object.fromEntries(Object.keys(v).sort().map(k=>[k,v[k]])):v);
const eq=(a,b)=>stable(a)===stable(b);
const record=x=>x!==null&&typeof x==='object'&&!Array.isArray(x);
const fail=message=>{throw Object.assign(new Error(message),{status:409})};
const keyed=x=>Array.isArray(x)&&x.every(v=>record(v)&&typeof v.id==='string'&&v.id)&&new Set(x.map(v=>v.id)).size===x.length;
function patch(current,from,to,path){
 if(eq(from,to))return copy(current);
 if(record(from)&&record(to)&&record(current)){
  const result=copy(current);
  for(const key of new Set([...Object.keys(from),...Object.keys(to)])){
   const value=patch(current[key],from[key],to[key],path+'.'+key);
   if(value===undefined)delete result[key];else Object.defineProperty(result,key,{value,writable:true,enumerable:true,configurable:true});
  }
  return result;
 }
 if(keyed(from)&&keyed(to)&&keyed(current)){
  const a=new Map(from.map(v=>[v.id,v])),b=new Map(to.map(v=>[v.id,v])),c=new Map(current.map(v=>[v.id,v]));
  for(const id of new Set([...a.keys(),...b.keys()])){const v=patch(c.get(id),a.get(id),b.get(id),path+'['+id+']');if(v===undefined)c.delete(id);else c.set(id,v)}
  let order=current.map(x=>x.id).filter(id=>c.has(id));
  // A concurrent independent insertion is retained; source reorders require unchanged relative order.
  const common=from.filter(x=>b.has(x.id)).map(x=>x.id),targetCommon=to.filter(x=>a.has(x.id)).map(x=>x.id);
  if(!eq(common,targetCommon)){
   const actual=current.filter(x=>a.has(x.id)&&b.has(x.id)).map(x=>x.id);
   if(!eq(actual,common))fail('History conflict at '+path+' order');
   let i=0;order=order.map(id=>a.has(id)&&b.has(id)?targetCommon[i++]:id);
  }
  for(let i=0;i<to.length;i++){
   const id=to[i].id;if(order.includes(id))continue;
   let index=-1;for(let j=i-1;j>=0&&index<0;j--)index=order.indexOf(to[j].id);
   order.splice(index+1,0,id);
  }
  return order.map(id=>c.get(id));
 }
 if(!eq(current,from))fail('History conflict at '+path+'; newer edits are preserved');
 return copy(to);
}
export function historyTransition({current,event,action,latest=null}){
 if(!['undo','redo'].includes(action)||!event||!['change','variants'].includes(event.operation))fail('Unsupported history operation');
 if(action==='undo'&&latest?.operation==='history.undo')fail('This change is already undone');
 if(action==='redo'&&latest?.operation!=='history.undo')fail('Undo first before redoing a change');
 for(const field of ['access','steering','steeringAudit','steeringReceipts'])if(!eq(event.before?.[field],event.after?.[field]))fail('History cannot change protected state');
 if(!record(event.before)||!record(event.after))fail('Historical snapshot unavailable');
 return action==='undo'?patch(current,event.after,event.before,'state'):patch(current,event.before,event.after,'state');
}
