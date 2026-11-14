'use strict';
function collectSummary(s,roots,opts={}){
 const eligible=s.items.filter(i=>(opts.hidden||!i.hidden)&&(opts.archived||!i.archived));
 const chosen=new Set(roots);
 if(opts.tree){
   const children=new Map();
   for(const i of eligible){if(!children.has(i.parent))children.set(i.parent,[]);children.get(i.parent).push(i.id)}
   const todo=[...chosen];
   for(let n=0;n<todo.length;n++){for(const id of children.get(todo[n])??[]){if(!chosen.has(id)){chosen.add(id);todo.push(id)}}}
 }
 const visible=eligible.filter(i=>chosen.has(i.id)), ids=new Set(visible.map(i=>i.id)), byId=new Map(visible.map(i=>[i.id,i]));
 const kids=new Map(), items=[], written=new Set();
 for(const i of visible){const parent=ids.has(i.parent)?i.parent:'';if(!kids.has(parent))kids.set(parent,[]);kids.get(parent).push(i)}
 const stack=[...(kids.get('')??[])].reverse();
 while(stack.length){const i=stack.pop();if(written.has(i.id))continue;written.add(i.id);items.push(i);stack.push(...[...(kids.get(i.id)??[])].reverse())}
 items.push(...visible.filter(i=>!written.has(i.id)));
 const depth=i=>{const seen=new Set([i.id]);let id=i.parent,n=0;while(ids.has(id)&&!seen.has(id)){seen.add(id);n++;id=byId.get(id)?.parent}return n};
 const states=Object.create(null);let priceCount=0,total=0;
 for(const i of items){states[i.state??'Unspecified']=(states[i.state??'Unspecified']||0)+1;if(i.kind==='item'&&Number.isFinite(i.price)){priceCount++;total+=i.price*(Number.isFinite(i.qty)?i.qty:1)}}
 const refs=s.evidence??[];
 return {items,depth,states,priceCount,total,refs,notes:s.notes??[]};
}
function summaryDialog(mode,id){
 const s=current(); if(!s)return;
 const tag=s.tags.find(t=>t.id===id);
 const roots=mode==='category'?s.items.filter(i=>i.tags.includes(id)).map(i=>i.id):mode==='selection'?[...selected]:[id];
 if(!roots.length){notice('Select one or more entries to summarize.');return}
 const name=mode==='category'?tag?.name:mode==='selection'?'Selected entries':byId(id)?.name;
 const prefsNow=prefs();
 const both=mode==='item'||mode==='category'||mode==='selection';
 const inner='<label><input type="checkbox" id="sum-tree" checked> Include nested options</label><label><input type="checkbox" id="sum-detail" '+(both?'checked':'')+'> Summarize each entry</label><label><input type="checkbox" id="sum-comments"> Include comment highlights</label><label><input type="checkbox" id="sum-archive" '+(prefsNow.archive?'':'disabled')+'> Include archived entries (enable in settings first)</label><div id="sum-preview" class="wide"></div>';
 dlg('Summary: '+(name||'Tracker'),inner,()=>{const txt=$('#sum-text')?.value||'';if(!txt)return false;const u=URL.createObjectURL(new Blob([txt],{type:'text/plain;charset=utf-8'}));const a=document.createElement('a');a.href=u;a.download='bedside-summary.txt';a.click();setTimeout(()=>URL.revokeObjectURL(u),3000);return false},'Download summary');
 const show=()=>{
   const r=collectSummary(s,roots,{tree:$('#sum-tree').checked,hidden:prefsNow.hidden,archived:prefsNow.archive&&$('#sum-archive').checked});
   const stateLine=Object.entries(r.states).map(([k,n])=>k+': '+n).join(' · ');
   const lines=[name||'Tracker',r.items.length+' entries · '+(stateLine||'No visible entries')];
   if(r.priceCount)lines.push('Recorded option totals: '+money(r.total)+' across '+r.priceCount+' priced entries. This is not a budget recommendation; alternatives can overlap.');
   if($('#sum-detail').checked){
     for(const i of r.items){
       const prefix='  '.repeat(Math.min(8,r.depth(i)));
       const notes=r.notes.filter(n=>n.targets?.includes(i.id));
       const refs=r.refs.filter(a=>a.targets?.includes(i.id));
       lines.push(prefix+'• '+i.name+' ['+i.state+']'+(i.kind==='item'&&i.price!==null?' · '+money(i.price)+' × '+i.qty:'')+(refs.length?' · '+refs.length+' market sources':''));
       if(i.description)lines.push(prefix+'  '+String(i.description).slice(0,260));
       if(i.url)lines.push(prefix+'  '+i.url);
       const attrs=(i.attributes??[]).slice(0,4).map(([k,v])=>k+': '+(typeof v==='string'?v:JSON.stringify(v)).slice(0,100));
       if(attrs.length)lines.push(prefix+'  '+attrs.join(' · '));
       if($('#sum-comments').checked)for(const n of notes.slice(-3))lines.push(prefix+'  '+(n.pref||'Comment')+': '+String(n.text).slice(0,180));
     }
   }
   $('#sum-preview').innerHTML='<label>Summary (select and copy, or download)<textarea id="sum-text" rows="15" readonly>'+e(lines.join('\n'))+'</textarea></label><small>Built from saved records. No web request was made.</small>';
 };
 form.querySelectorAll('#sum-tree,#sum-detail,#sum-comments,#sum-archive').forEach(x=>x.addEventListener('change',show));show();
}
