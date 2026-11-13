'use strict';
let reportingTimeZone='America/Chicago';
function reportingReceiptButton(){return button('report-receipt','Stage receipt / purchases')}
function reportingScopes(s){return scopes().map(([id,label])=>({id,label,archived:!!archivedScope(id,s),ancestors:id.startsWith('item:')?['global',...(s.items.find(x=>x.id===id.slice(5))?.tags??[]).map(x=>'tag:'+x)]:['global']}))}
function reportingRefresh(ids){apply('Reviewed recommended defaults',s=>Object.assign(s,BedawayReports.previewDefaults(s,{scopes:reportingScopes(s).filter(x=>ids.includes(x.id)),sections:Object.keys(defaults),revision:version+1})))}
function reportingAccept(ids){apply('Accepted current default previews',s=>Object.assign(s,BedawayReports.acceptDefaults(s,{scopes:reportingScopes(s),ids,revision:version})))}
function reportingInventory(id){
 const i=byId(id);if(!i||!editable())return;
 dlg('Log stock or spending',select('Action','mode',['Bought','Consumed','Stock adjustment','Expense'],'Bought')+field('Quantity','qty',1,'number')+field('Quantity unit','unit','units')+field('Total paid (USD, optional)','total','','number')+field('Store','store',i.store)+field('Date','at',new Date().toLocaleDateString('en-CA'),'date')+select('Day context','context',['Unspecified','Holiday','Busy','Slow'],'Unspecified')+area('Allocation (optional): category ID | dollars per line','allocations','')+'<p class="wide">'+current().tags.map(t=>e(t.id)+' = '+e(t.name)).join(' · ')+'<br>Leave blank for an exact-cent equal split across categories. An unknown amount remains unknown. Adjustments can be negative; consumed quantity must be positive.</p>'+area('Reason / receipt reference','note'),f=>{
  const qty=Number(f.qty),totalMinor=BedawayReports.moneyToMinor(f.total);if(!Number.isFinite(qty)||f.mode!=='Stock adjustment'&&qty<0)throw new Error('Invalid quantity');
  const allocations=f.allocations.trim()?f.allocations.split('\n').filter(x=>x.trim()).map(x=>{const a=x.split('|');if(a.length!==2||!current().tags.some(t=>t.id===a[0].trim()))throw new Error('Use an existing category ID | dollars');return {categoryId:a[0].trim(),minor:BedawayReports.moneyToMinor(a[1].trim())}}):null;
  if(allocations&&totalMinor===null)throw new Error('Enter a total before allocating');if(allocations)BedawayReports.allocateExpense(totalMinor,[],allocations);
  apply('Logged '+f.mode+' for '+i.name,s=>s.inventory.push({id:uid(),item:id,name:i.name,owner:i.owner,tags:[...i.tags],author:hub.user.id,mode:f.mode,qty,totalMinor,total:totalMinor===null?null:totalMinor/100,currency:'USD',allocations,unit:tidy(f.unit)||'units',store:tidy(f.store)||'Unspecified',note:prose(f.note),context:f.context,at:f.at||new Date().toISOString(),recorded:new Date().toISOString()}));
 });
}
function reportingReceipt(){
 if(!editable()){notice('Editor access is required to stage purchases.');return;}
 const storeId=active.id;
 dlg('Stage a receipt',field('Merchant / store','store','')+field('Purchase date','at',new Date().toLocaleDateString('en-CA'),'date')+select('Currency','currency',['USD','EUR','GBP','CAD','AUD'],'USD')+area('One line: item name | total paid | quantity','receipt','')+'<p class="wide">Paste transcribed receipt lines. This does not OCR images or fetch merchant data. Each line total is for the full quantity, not unit price. Blank amounts remain unknown. Review matches before saving.</p>',f=>{
  const stage=BedawayReports.stageReceipt(f.receipt,{id:uid(),store:tidy(f.store),at:f.at,items:current().items,currency:f.currency});
  reportingReceiptReview(stage,storeId,version);return false;
 },'Review lines');
}
function reportingReceiptReview(stage,storeId,baseRevision){
 const choices=[['','Choose an existing item'],...current().items.filter(i=>!i.archived&&i.kind==='item').map(i=>[i.id,i.name])];
 dlg('Review receipt lines',stage.lines.map((line,n)=>'<section class="wide panel"><label><input type="checkbox" name="take:'+n+'" '+(line.itemId?'checked':'')+'> '+e(line.name)+' · '+e(line.totalMinor===null?'Amount unknown':stage.currency+' '+(line.totalMinor/100).toFixed(2))+' · quantity '+e(line.quantity)+'</label>'+select('Match to item','match:'+n,choices,line.itemId||'')+'</section>').join('')+'<p class="wide">Only checked lines are added to the inventory ledger. Unmatched lines are retained in this preview, not silently created or discarded. Cancel makes no changes.</p>',f=>{
  if(active.id!==storeId||version!==baseRevision)throw new Error('Store changed. Cancel and stage again against the latest data.');
  const selectedIds=stage.lines.filter((line,n)=>f['take:'+n]).map(line=>line.id);if(!selectedIds.length)return false;
  const reviewed=copy(stage);reviewed.lines.forEach((line,n)=>line.itemId=f['match:'+n]||null);
  apply('Imported reviewed receipt',s=>Object.assign(s,BedawayReports.applyReceipt(s,reviewed,{selected:selectedIds,revision:version,expectedRevision:baseRevision,author:hub.user.id})));
 },'Save checked purchases');
}
function reportingView(s){
 const groupOptions=['Category','Store','Person','Author','Author / Category','Category / Author','Action','Hour','Day of month','Day of week','Calendar date','Holiday / busy / slow'];
 const r=BedawayReports.buildReport(s,{groupBy:reportGroup,timeZone:reportingTimeZone});
 const format=(minor,currency)=>new Intl.NumberFormat('en-US',{style:'currency',currency}).format(minor/100);
 return '<h2>Inventory & spend</h2><div class="row">'+reportingReceiptButton()+button('budget','Set budget')+button('research','Research spending','report|spend')+'</div><section class="panel"><h3>Recorded spend</h3>'+Object.entries(r.totals).map(([currency,total])=>'<p class="metric">'+e(format(total,currency))+(currency==='USD'&&s.budget?' / '+e(money(s.budget))+' budget':'')+'</p>').join('')+'<p>'+r.unknownAmountCount+' records have unknown amounts. Currencies are never silently combined or converted.</p><label>Group by<select id="reportgroup">'+opt(groupOptions,reportGroup)+'</select></label><label>Report time zone<input id="report-timezone" value="'+e(reportingTimeZone)+'"></label><p>Dates without times stay dates, and their hour stays unknown. Holiday/busy/slow labels are recorded by people.</p>'+r.groups.sort((a,b)=>b.minor-a.minor).map(g=>'<p>'+e(g.key)+' · '+e(format(g.minor,g.currency))+' · '+g.recordCount+' records'+(g.unknownAmountCount?' ('+g.unknownAmountCount+' unknown amounts)':'')+'<span class="bar" style="width:'+Math.max(0,Math.min(100,r.totals[g.currency]?g.minor/r.totals[g.currency]*100:0))+'%"></span></p>').join('')+'<small>Uses explicit allocations where recorded; otherwise categories receive an exact-cent equal split. Each transaction contributes its total only once.</small></section><div class="grid"><section class="panel"><h3>On hand</h3>'+r.stock.map(x=>'<p>'+e(x.name)+' · '+e(x.quantity)+' '+e(x.unit)+(x.quantity<0?' · needs reconciliation':'')+'</p>').join('')+'</section><section class="panel"><h3>Activity & receipts</h3>'+[...s.inventory].reverse().map(x=>'<p><b>'+e(x.mode)+' · '+e(x.name)+'</b><br>'+e(x.at)+' · '+e(x.author)+' · '+e(x.store)+'<br>'+e(x.note)+'</p>').join('')+'</section></div>';
}
document.addEventListener('click',ev=>{const b=ev.target.closest('button');if(b?.dataset.act==='report-receipt')reportingReceipt()});
document.addEventListener('change',ev=>{if(ev.target.id==='report-timezone'){try{new Intl.DateTimeFormat('en-US',{timeZone:ev.target.value});reportingTimeZone=ev.target.value;render()}catch{notice('Use an IANA time zone such as America/Chicago.')}}});
function reportingDecisionButton(){return button('report-group-decision','Compare for multiple people')}
function reportingGroupDecision(){
 if(!editable()){notice('Editor access is required to record a comparison.');return;}
 const s=current(),context=decisionContext(s),people=s.people||[],items=context.items;
 if(!items.length||!people.length){notice('Add people and comparison candidates first.');return;}
 dlg('Multi-person comparison',select('Candidate','item',items.map(i=>[i.id,i.name]),items[0].id)+select('Person label (manual assessment)','person',people,people[0])+'<p class="wide">These are manually entered assessments for a person label, not authenticated votes by that person. Unknowns stay unknown. A required constraint blocks eligibility unless explicitly satisfied.</p>'+context.criteria.map(c=>select(c.name,'rating:'+c.id,[['','Unknown'],...[0,1,2,3,4,5].map(x=>[String(x),String(x)])],'')).join('')+select('Required fit / compatibility constraint','constraint',[['unknown','Not verified'],['yes','Satisfied'],['no','Not satisfied'],['none','Not required']],'unknown')+area('Evidence and reasoning','note'),f=>{
  apply('Recorded multi-person assessment',a=>{const b=decisionBoard(a,context.key);b.personRatings??=[];const row={itemId:f.item,person:f.person,scores:Object.fromEntries(context.criteria.filter(c=>f['rating:'+c.id]!=='').map(c=>[c.id,Number(f['rating:'+c.id])])),constraints:f.constraint==='none'?[]:[{required:true,met:f.constraint==='unknown'?null:f.constraint==='yes'}],note:prose(f.note),recordedBy:hub.user.id,at:new Date().toISOString()};b.personRatings=b.personRatings.filter(x=>x.itemId!==row.itemId||x.person!==row.person);b.personRatings.push(row)});
 });
}
function reportingGroupSummary(s){
 const {board,criteria,items}=decisionContext(s),people=s.people??[];
 if(!board?.personRatings?.length)return '';
 return '<section class="panel"><h3>Multi-person assessments</h3><p>Equal weight per person label. Missing assessments prevent a combined score; fit eligibility is separate from preference.</p>'+items.map(i=>{
  const rows=people.map(person=>{const r=board.personRatings.find(x=>x.itemId===i.id&&x.person===person);return {id:person,scores:r?.scores??{},constraints:r?.constraints??[{required:true,met:null}]}}),result=BedawayReports.groupDecision(criteria,rows);
  return '<p><b>'+e(i.name)+'</b> · '+(result.score===null?'Incomplete ratings':result.score.toFixed(0)+'/100')+' · '+(result.eligible===true?'constraints satisfied':result.eligible===false?'required constraint failed':'constraints unverified')+'</p>';
 }).join('')+'</section>';
}
document.addEventListener('click',ev=>{const b=ev.target.closest('button');if(b?.dataset.act==='report-group-decision')reportingGroupDecision()});
