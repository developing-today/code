'use strict';
// Families describe lasting associations; parent describes only the current layout.
function variantFamilies(item){return [...new Set([...(item.variantFamilyIds||[]),item.variantFamilyId].filter(Boolean))]}
function variantPlan(state,ids,options,newId){
 const staged=JSON.parse(JSON.stringify(state));
 const result=variantPlanStaged(staged,ids,options,newId);
 state.items=staged.items;return result;
}
function variantPlanStaged(state,ids,options,newId){
 const map=new Map();for(const item of state.items){if(!item.id||map.has(item.id))throw Error('Duplicate or invalid item ID.');map.set(item.id,item)}
 for(const item of state.items){let p=item.parent,seen=new Set([item.id]);while(p){if(seen.has(p))throw Error('This hierarchy contains a cycle.');seen.add(p);const parent=map.get(p);if(!parent||parent.kind!=='virtual')throw Error('Parent must be an existing general item.');p=parent.parent}}
 const allocated=new Set(map.keys());const allocate=newId;newId=()=>{const id=allocate();if(typeof id!=='string'||!id||allocated.has(id))throw Error('ID allocation failed.');allocated.add(id);return id};
 const destinationParent=options.destinationParent||'';
 if(destinationParent&&(!map.has(destinationParent)||map.get(destinationParent).kind!=='virtual'||map.get(destinationParent).archived))throw Error('Choose an active destination parent.');
 const selectedIds=[...new Set(ids)],chosen=selectedIds.map(id=>state.items.find(x=>x.id===id));
 if(!chosen.length||chosen.some(x=>!x))throw Error('Select existing items first.');
 const selectedSet=new Set(selectedIds);
 for(const item of chosen){let parent=item.parent,seen=new Set();while(parent){if(selectedSet.has(parent))throw Error('Select either a group or its children, not both.');if(seen.has(parent))throw Error('This hierarchy contains a cycle.');seen.add(parent);parent=state.items.find(x=>x.id===parent)?.parent}}
 if(!['group','split','detach'].includes(options.action))throw Error('Unknown variant action.');
 let target=options.target?state.items.find(x=>x.id===options.target):null;
 if(options.action==='group'&&options.target&&(!target||target.kind!=='virtual'||target.archived))throw Error('Choose an active general item.');
 if(destinationParent){let p=destinationParent,seen=new Set();while(p){if(selectedSet.has(p))throw Error('A group cannot be placed inside itself or its descendants.');if(seen.has(p))throw Error('This hierarchy contains a cycle.');seen.add(p);p=map.get(p)?.parent}}
 if(target){let p=target.id,seen=new Set();while(p){if(selectedSet.has(p))throw Error('A group cannot be placed inside itself or its descendants.');if(seen.has(p))throw Error('This hierarchy contains a cycle.');seen.add(p);p=state.items.find(x=>x.id===p)?.parent}}
 if(options.action==='group'&&!target&&!String(options.name||'').trim())throw Error('Name the new general item.');
 const inherited=new Set(chosen.flatMap(item=>[...variantFamilies(item),...variantFamilies(state.items.find(x=>x.id===item.parent)||{})]));
 if(!inherited.size)inherited.add(newId());
 const groups=[];
 function createGroup(name){if(!options.allowDuplicateName&&state.items.concat(groups).some(x=>String(x.name||'').normalize('NFC').trim().toLocaleLowerCase()===name.normalize('NFC').trim().toLocaleLowerCase()))throw Error('A name already exists; explicitly allow duplicate names.');const group={id:newId(),name,kind:'virtual',owner:'Shared',tags:[...new Set(chosen.flatMap(x=>x.tags||[]))],parent:destinationParent,url:'',state:'Considering',hidden:false,archived:false,qty:1,price:null,store:'',attributes:[],description:'',variantFamilyId:newId(),variantFamilyIds:[...inherited],variantSchemaVersion:1};groups.push(group);return group}
 if(options.action==='group'&&!target)target=createGroup(String(options.name).trim());
 const assignments=new Map();
 if(options.action==='split'){for(const item of chosen){const name=String(options.labels?.[item.id]||'').trim();if(name&&!assignments.has(name))assignments.set(name,createGroup(name))}}
 // Validation above happens before any mutation, including family normalization.
 if(target){target.variantFamilyId||=newId();target.variantSchemaVersion=1;target.variantFamilyIds=[...new Set([...variantFamilies(target),...inherited])];}
 for(const item of chosen){const destination=options.action==='group'?target:options.action==='split'?assignments.get(String(options.labels?.[item.id]||'').trim()):null;
 item.variantFamilyIds=[...new Set([...variantFamilies(item),...inherited,...(destination?variantFamilies(destination):[])])];item.variantSchemaVersion=1;item.parent=destination?.id||destinationParent;}
 state.items.push(...groups);
 if(options.pruneFamilyIds){const prune=new Set(options.pruneFamilyIds);for(const item of chosen){item.variantFamilyIds=variantFamilies(item).filter(x=>!prune.has(x));if(prune.has(item.variantFamilyId))delete item.variantFamilyId;}}
 return {itemIds:selectedIds,groupIds:groups.map(x=>x.id),familyIds:[...inherited]};
}
function variantsDialog(id=''){
 if(!editable()){notice('Editor access is required to regroup variants.');return}
 const state=current(),origin=byId(id),candidates=id?(origin?.kind==='virtual'?state.items.filter(x=>x.parent===id):origin?[origin]:[]):[...selected].map(byId).filter(Boolean);
 if(!candidates.length){notice('Select items in the tracker, or open Variants on a general item containing options.');return}
 const options=state.items.filter(x=>x.kind==='virtual'&&!x.archived).map(x=>[x.id,x.name]);
 dlg('Group or split variants','<p class="wide">Existing item IDs, notes, ratings and history stay intact. Family IDs preserve associations even when options stand alone. Choose items below; split labels create separate new sets, while blank labels make standalone items. Existing empty groups are kept.</p>'+select('Action','variantAction',[['group','Group under one general item'],['split','Split into named sets / standalone items'],['detach','Make selected items standalone']],'group')+select('General item (group action)','variantTarget',[['','Create a new general item'],...options],'')+field('New general item name','variantName','')+candidates.map(x=>'<div class="wide"><label><input type="checkbox" name="variantPick:'+e(x.id)+'" checked> '+e(x.name)+'</label><small>Item ID: '+e(x.id)+' · '+variantFamilies(x).length+' family association(s)</small>'+field('New set label (split action; blank = standalone)','variantSet:'+x.id,'')+'</div>').join(''),f=>{
 const ids=candidates.filter(x=>f['variantPick:'+x.id]).map(x=>x.id),action=f.variantAction,labels=Object.fromEntries(ids.map(i=>[i,tidy(f['variantSet:'+i]||'')]));
 const options={action,target:action==='group'?f.variantTarget:'',name:tidy(f.variantName),labels};
 // Validate synchronously so the dialog stays open on errors. apply supplies the versioned, undoable event.
 variantPlan(copy(current()),ids,options,uid);
 void variantCommand(ids,options).catch(err=>notice(err.message));
 });
}
document.addEventListener('click',ev=>{const b=ev.target.closest('button');if(b?.dataset.act==='variants')variantsDialog(b.dataset.id||'')});
