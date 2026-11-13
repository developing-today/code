'use strict';
/* Pure policy/data helpers. Call only on the caller's authorized projection. */
globalThis.BedawayReports=(()=>{
 const clone=x=>JSON.parse(JSON.stringify(x));
 const own=(x,k)=>Object.prototype.hasOwnProperty.call(x??{},k);
 const safe=x=>typeof x==='string'&&x.length>0&&x.length<200&&!['__proto__','prototype','constructor'].includes(x);
 const unique=xs=>[...new Set(xs)].sort();
 function moneyToMinor(value){
  if(value===null||value===undefined||value==='')return null;
  const text=String(value).trim();if(!/^\d+(?:\.\d{1,2})?$/.test(text))throw new Error('Use a nonnegative amount with at most two decimal places');
  const [whole,fraction='']=text.split('.'),result=Number(whole)*100+Number(fraction.padEnd(2,'0'));
  if(!Number.isSafeInteger(result))throw new Error('Amount is too large');return result;
 }
 function allocateExpense(minor,categories=[],explicit=null){
  if(!Number.isSafeInteger(minor)||minor<0)throw new Error('Invalid minor-unit total');
  if(explicit){
   if(!Array.isArray(explicit)||!explicit.length||explicit.some(x=>!safe(x.categoryId)||!Number.isSafeInteger(x.minor)||x.minor<0)||new Set(explicit.map(x=>x.categoryId)).size!==explicit.length||explicit.reduce((s,x)=>s+x.minor,0)!==minor)throw new Error('Allocations must be unique and sum exactly to the total');
   return explicit.map(x=>({categoryId:x.categoryId,minor:x.minor}));
  }
  const ids=unique(categories.filter(safe));if(!ids.length)ids.push('Uncategorized');
  return ids.map((categoryId,i)=>({categoryId,minor:Math.floor(minor/ids.length)+(i<minor%ids.length?1:0)}));
 }
 function stageReceipt(text,{id,store='',at='',items=[],currency='USD'}={}){
  if(!safe(id)||!['USD','EUR','GBP','CAD','AUD'].includes(currency))throw new Error('Receipt ID and supported two-decimal currency are required');
  if(typeof text!=='string'||text.length>100000)throw new Error('Receipt text too long');
  const lines=text.split(/\r?\n/).filter(x=>x.trim()).map((raw,index)=>{
   const parts=raw.split('|').map(x=>x.trim());if(parts.length>3)throw new Error('Use name | total | quantity');
   const name=parts[0],totalMinor=moneyToMinor(parts[1]??null),quantity=parts[2]?Number(parts[2]):1;
   if(!name||!Number.isFinite(quantity)||quantity<=0)throw new Error('Name and positive quantity required');
   const matches=items.filter(x=>x.name.normalize('NFC').toLocaleLowerCase()===name.normalize('NFC').toLocaleLowerCase());
   return {id:id+':'+index,name,raw,totalMinor,quantity,itemId:matches.length===1?matches[0].id:null,match:matches.length===1?'exact-name':matches.length?'ambiguous':'unmatched'};
  });
  if(lines.length>500)throw new Error('At most 500 receipt lines');
  return {schema:'bedaway.receipt',version:1,id,status:'pending',store,at,currency,sourceText:text,lines};
 }
 function applyReceipt(state,stage,{selected=[],revision,expectedRevision,author}={}){
  if(revision!==expectedRevision)throw new Error('Store changed; review the receipt again');
  if(stage.status!=='pending'||!safe(stage.id)||!safe(author))throw new Error('Pending receipt and authenticated author required');
  const next=clone(state),known=new Set((next.inventory??=[]).map(x=>x.id)),ids=new Set(selected);
  for(const line of stage.lines){if(!ids.has(line.id))continue;
   const recordId='receipt:'+line.id;if(known.has(recordId))continue;
   const item=next.items.find(x=>x.id===line.itemId);if(!item)throw new Error('Match every selected line to an existing item first');
   if(!Number.isFinite(line.quantity)||line.quantity<=0||line.totalMinor!==null&&(!Number.isSafeInteger(line.totalMinor)||line.totalMinor<0))throw new Error('Invalid receipt line');
   next.inventory.push({id:recordId,receiptId:stage.id,item:item.id,name:item.name,owner:item.owner??'',tags:[...item.tags],author,mode:'Bought',qty:line.quantity,totalMinor:line.totalMinor,total:line.totalMinor===null?null:line.totalMinor/100,currency:stage.currency,store:stage.store||'Unspecified',at:stage.at,recorded:new Date().toISOString(),note:line.raw,context:'Unspecified'});known.add(recordId);
  }return next;
 }
 function timeParts(at,timeZone){
  if(/^\d{4}-\d{2}-\d{2}$/.test(at??''))return {date:at,day:at.slice(-2),weekday:new Intl.DateTimeFormat('en-US',{weekday:'long',timeZone:'UTC'}).format(new Date(at+'T12:00:00Z')),hour:'Unknown hour'};
  const date=new Date(at);if(!at||!Number.isFinite(date.getTime()))return {date:'Unknown date',day:'Unknown day',weekday:'Unknown weekday',hour:'Unknown hour'};
  const values=Object.fromEntries(new Intl.DateTimeFormat('en-US',{timeZone,year:'numeric',month:'2-digit',day:'2-digit',weekday:'long',hour:'2-digit',hourCycle:'h23'}).formatToParts(date).map(x=>[x.type,x.value]));
  return {date:values.year+'-'+values.month+'-'+values.day,day:values.day,weekday:values.weekday,hour:values.hour+':00'};
 }
 function buildReport(state,{groupBy='Category',timeZone='UTC'}={}){
  new Intl.DateTimeFormat('en-US',{timeZone}); // reject invalid zones rather than silently using the viewer's zone
  const totals={},groups=new Map(),stock=new Map();let unknownAmountCount=0;
  for(const r of state.inventory??[]){
   const currency=r.currency||'USD';if(!/^[A-Z]{3}$/.test(currency))throw new Error('Invalid currency');
   const minor=own(r,'totalMinor')?r.totalMinor:moneyToMinor(r.total);
   if(minor!==null&&(!Number.isSafeInteger(minor)||minor<0))throw new Error('Invalid recorded total');
   const t=timeParts(r.at,timeZone),categories=allocateExpense(minor??0,r.tags??[],r.allocations??null);
   const categoryName=id=>state.tags.find(x=>x.id===id)?.name??id;
   const dimensions={'Store':r.store||'Unspecified','Person':r.owner||'Unspecified','Author':r.author||'Unspecified','Action':r.mode||'Unspecified','Hour':t.hour,'Day of month':t.day,'Day of week':t.weekday,'Calendar date':t.date,'Holiday / busy / slow':r.context||'Unspecified'};
   let entries;
   if(groupBy==='Category')entries=categories.map(x=>[categoryName(x.categoryId),x.minor,x.categoryId]);
   else if(['Author / Category','Category / Author'].includes(groupBy))entries=categories.map(x=>[groupBy.startsWith('Author')?[r.author||'Unspecified',categoryName(x.categoryId)].join(' / '):[categoryName(x.categoryId),r.author||'Unspecified'].join(' / '),x.minor,JSON.stringify([r.author,x.categoryId])]);
   else {if(!own(dimensions,groupBy))throw new Error('Unknown reporting dimension');entries=[[dimensions[groupBy],minor??0,dimensions[groupBy]]];}
   if(minor===null)unknownAmountCount++;else totals[currency]=(totals[currency]??0)+minor;
   for(const [key,amount,identity]of entries){const k=JSON.stringify([identity,currency]);if(!groups.has(k))groups.set(k,{key,currency,minor:0,recordCount:0,unknownAmountCount:0});const g=groups.get(k);g.minor+=amount;g.recordCount++;if(minor===null)g.unknownAmountCount++;}
   if(r.mode!=='Expense'&&r.item&&Number.isFinite(r.qty)){const unit=r.unit||'units',k=JSON.stringify([r.item,unit]);if(!stock.has(k))stock.set(k,{itemId:r.item,name:r.name,unit,quantity:0});stock.get(k).quantity+=r.mode==='Consumed'?-Math.abs(r.qty):r.qty;}
  }return {totals,groups:[...groups.values()],stock:[...stock.values()],unknownAmountCount,timeZone};
 }
 function matches(pattern,value){
  if(typeof pattern!=='string'||pattern.length>200)return false;
  return new RegExp('^'+pattern.replace(/[.+^${}()|[\]\\]/g,'\\$&').replace(/\*/g,'.*').replace(/\?/g,'.')+'$','u').test(value);
 }
 function policy(state,scope,section){
  const lineage=unique(['global',...(scope.ancestors??[])]).filter(x=>x!==scope.id); // preserve caller ancestor order below
  const ordered=[...new Set(['global',...(scope.ancestors??[]),scope.id])];
  let rule={},explicit=null,disabled=false;
  for(const id of ordered){const r=state.rules?.[id];if(r)rule={...rule,...r};const v=state.overrides?.[id]?.[section];if(v==='disabled'||r?.mode==='off')disabled=true;if(v!==undefined)explicit=v;}
  return {rule,explicit,disabled};
 }
 function previewDefaults(state,{scopes,sections,revision}={}){
  const next=clone(state);next.recs??={};next.overrides??={};
  for(const scope of scopes){if(!safe(scope.id)||scope.archived)continue;for(const section of sections){
   if(!safe(section))throw new Error('Invalid section');const id=scope.id+'|'+section,previous=next.recs[id];
   if(['cancelled','accepted'].includes(previous?.state))continue;
   const {rule,explicit,disabled}=policy(next,scope,section),token=(scope.label||scope.id)+'/'+section;
   if(rule.sections?.length&&!rule.sections.includes(section)||rule.include?.length&&!rule.include.some(p=>matches(p,token)||matches(p,section))||rule.exclude?.some(p=>matches(p,token)||matches(p,section)))continue;
   const status=disabled?'recommended · disabled':explicit!==null?'custom override':rule.mode==='on'?'accepted':'pending';
   next.recs[id]={id,scope:scope.id,section,state:status,baseRevision:revision};
   if(status==='accepted'){next.overrides[scope.id]??={};next.overrides[scope.id][section]='enabled';}
  }}return next;
 }
 function acceptDefaults(state,{scopes,revision,ids=null}={}){
  const next=clone(state);next.overrides??={};
  for(const r of Object.values(next.recs??{})){
   if(r.state!=='pending'||ids&&!ids.includes(r.id))continue;
   const scope=scopes.find(x=>x.id===r.scope);if(!scope||scope.archived)continue;
   if(r.baseRevision!==revision){r.state='stale';continue;}
   const {rule,explicit,disabled}=policy(next,scope,r.section);if(disabled||explicit!==null)continue;
   const token=(scope.label||scope.id)+'/'+r.section;
   if(rule.sections?.length&&!rule.sections.includes(r.section)||rule.include?.length&&!rule.include.some(p=>matches(p,token)||matches(p,r.section))||rule.exclude?.some(p=>matches(p,token)||matches(p,r.section)))continue;
   next.overrides[r.scope]??={};next.overrides[r.scope][r.section]='enabled';r.state='accepted';
  }return next;
 }
 function groupDecision(criteria,people){
  const rows=people.map(person=>{
   let sum=0,total=0,known=0;for(const c of criteria){if(!Number.isFinite(c.weight)||c.weight<=0)continue;total+=c.weight;const v=person.scores?.[c.id];if(typeof v==='number'&&Number.isFinite(v)&&v>=0&&v<=5){known+=c.weight;sum+=c.weight*v;}}
   const constraints=(person.constraints??[]).filter(x=>x.required);return {id:person.id,score:total>0&&known===total?100*sum/(5*total):null,coverage:total?known/total:0,eligible:constraints.some(x=>x.met===false)?false:constraints.some(x=>x.met!==true)?null:true};
  });
  return {people:rows,score:rows.length&&rows.every(x=>x.score!==null)?rows.reduce((s,x)=>s+x.score,0)/rows.length:null,eligible:rows.some(x=>x.eligible===false)?false:rows.some(x=>x.eligible===null)?null:rows.length?true:null};
 }
 return {moneyToMinor,allocateExpense,stageReceipt,applyReceipt,buildReport,previewDefaults,acceptDefaults,groupDecision};
})();
