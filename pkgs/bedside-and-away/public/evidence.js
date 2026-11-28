'use strict';
// Source evidence is a top-level snapshot collection, never private fields inside public item projections.
function evidenceExtract(text){
 const raw=String(text??'').slice(0,12000),lines=raw.split(/\r?\n/).map(x=>x.trim()).filter(Boolean);
 const url=raw.match(/https?:\/\/[^\s<>"']+/i)?.[0]?.replace(/[),.;]+$/,'')||'';
 const price=raw.match(/(?:USD\s*|US\$\s*|\$\s*)(\d[\d,]*(?:\.\d{1,2})?)/i)?.[1];
 const rating=raw.match(/\b([0-5](?:\.\d+)?)\s*(?:out of\s*5|\/\s*5|stars?)/i)?.[1];
 const count=raw.match(/\b([\d,]+)\s*(?:global\s+)?(?:ratings?|reviews?)\b/i)?.[1];
 const title=lines.find(x=>!/^https?:\/\//i.test(x)&&!/^\$\d/.test(x))||'';
 return {title:tidy(title).slice(0,200),url:link(url),price:price===undefined?'':Number(price.replaceAll(',','')),rating:rating===undefined?'':Number(rating),reviewCount:count===undefined?'':Number(count.replaceAll(',',''))};
}
let evidencePhotoDraft=null,evidencePhotoBusy=false,evidenceDialogNonce=0;
function evidenceImageAllowed(p){return !!p&&typeof p.data==='string'&&p.data.length<=70000&&/^data:image\/(?:jpeg|png|webp);base64,[A-Za-z0-9+/]+=*$/.test(p.data)}
function evidenceEditor(ids){
 if(!editable()){notice('Choose a store you can edit before attaching evidence.');return}
 ids=[...new Set(ids||[])].filter(x=>typeof x==='string'&&x);if(!ids.length){notice('Select one or more entries first.');return}
 evidencePhotoDraft=null;evidencePhotoBusy=false;const nonce=++evidenceDialogNonce;
 dlg('Attach market evidence to '+ids.length+' '+(ids.length===1?'entry':'entries'),
 '<p class="wide">Save a listing, review, picture or reference with its source. Nothing is fetched automatically. Do not paste passwords, cookies, authentication tokens or payment details.</p>'+
 select('Evidence type','evidenceType',['listing','review','photo','reference'],'listing')+field('Title','title')+field('Source URL','url','','url')+field('Marketplace / publisher','market')+
 area('Paste listing or review text to extract fields','capture')+'<div class="wide">'+button('evidence-extract','Extract into empty fields')+'<small>Local pattern extraction only. Review the values before saving.</small></div>'+
 field('Observed price (USD, optional)','price','','number')+field('Rating out of 5 (optional)','rating','','number')+field('Review count (optional)','reviewCount','','number')+field('Source author / reviewer (optional)','sourceAuthor')+
 field('Picture source link (opens only when clicked)','photoUrl','','url')+'<label class="wide">Or attach your picture <input name="picture" type="file" accept="image/jpeg,image/png,image/webp"></label><div id="evidence-photo-status" class="wide"><small>JPEG, PNG or WebP, up to 5 MB input. Locally resized to a small preview; the original is not uploaded. Review screenshots for personal information first.</small></div>'+
 area('Your comment / core details','comment'), f=>{
  if(evidencePhotoBusy){alert('Wait for the picture preview to finish.');return false}
  const title=tidy(f.title),url=link(f.url),photoUrl=link(f.photoUrl);
  if(!title){alert('Give the evidence a title.');return false}
  if((f.url&&!url)||(f.photoUrl&&!photoUrl)){alert('Use HTTP or HTTPS source links.');return false}
  if(f.price!==''&&(!Number.isFinite(+f.price)||+f.price<0)||f.rating!==''&&(!Number.isFinite(+f.rating)||+f.rating<0||+f.rating>5)||f.reviewCount!==''&&(!Number.isInteger(+f.reviewCount)||+f.reviewCount<0)){alert('Check the price, 0–5 rating, and whole-number review count.');return false}
  const picture=evidencePhotoDraft;
  const total=(current().evidence??[]).reduce((n,r)=>n+(r.picture?.data?.length||0),0)+(picture?.data?.length||0);
  if(total>280000){alert('This store reached its picture-preview budget. Use a picture link instead.');return false}
  const record={schema:'bedaway.market-evidence',version:1,id:uid(),targets:ids,type:f.evidenceType,title:title.slice(0,200),url,market:tidy(f.market).slice(0,120),price:f.price===''?null:+f.price,currency:'USD',rating:f.rating===''?null:+f.rating,reviewCount:f.reviewCount===''?null:+f.reviewCount,sourceAuthor:tidy(f.sourceAuthor).slice(0,160),photoUrl,picture,comment:prose(f.comment).slice(0,6000),capture:prose(f.capture).slice(0,12000),capturedAt:new Date().toISOString(),author:userSettings.nickname||'Member',device,archived:false,verification:'User-supplied, not independently verified'};
  apply('Attached market evidence: '+record.title,s=>(s.evidence??=[]).push(record));
 },'Save reviewed evidence');
 const input=form.elements.picture;
 input.onchange=async()=>{evidencePhotoDraft=null;const file=input.files?.[0],status=document.getElementById('evidence-photo-status');if(!file){status.textContent='No picture selected.';return}
  if(file.size>5*1024*1024||!['image/jpeg','image/png','image/webp'].includes(file.type)){status.textContent='Choose JPEG, PNG or WebP no larger than 5 MB.';input.value='';return}
  evidencePhotoBusy=true;status.textContent='Preparing a small local preview…';let objectUrl;
  try{objectUrl=URL.createObjectURL(file);const image=new Image();image.src=objectUrl;await image.decode();if(nonce!==evidenceDialogNonce)return;
   const ratio=Math.min(1,640/image.naturalWidth,640/image.naturalHeight),canvas=document.createElement('canvas');canvas.width=Math.max(1,Math.round(image.naturalWidth*ratio));canvas.height=Math.max(1,Math.round(image.naturalHeight*ratio));const ctx=canvas.getContext('2d');if(!ctx)throw Error('Image preview is unavailable');ctx.fillStyle='#ffffff';ctx.fillRect(0,0,canvas.width,canvas.height);ctx.drawImage(image,0,0,canvas.width,canvas.height);
   let data;for(const quality of [.75,.55,.35]){data=canvas.toDataURL('image/jpeg',quality);if(data.length<=70000)break}if(data.length>70000)throw Error('This picture is too detailed for a small preview. Use its source link or a smaller screenshot.');
   evidencePhotoDraft={data,width:canvas.width,height:canvas.height,name:tidy(file.name).slice(0,160)};status.innerHTML='<img alt="Picture attachment preview" style="max-width:100%;max-height:200px" src="'+e(data)+'"><p>Small JPEG preview ready. Metadata was not copied.</p>';
  }catch(err){if(nonce===evidenceDialogNonce){evidencePhotoDraft=null;status.textContent=err.message;input.value=''}}finally{if(objectUrl)URL.revokeObjectURL(objectUrl);if(nonce===evidenceDialogNonce)evidencePhotoBusy=false}
 };
}
function evidenceMarkup(id){
 const list=(current().evidence??[]).filter(r=>r.targets?.includes(id)&&(!r.archived||prefs().archive));if(!list.length)return '';
 return '<details><summary>Market evidence ('+list.length+')</summary>'+list.map(r=>'<section class="panel"><div class="eyebrow">'+e(r.type)+(r.archived?' · Archived':'')+'</div><h3>'+e(r.title)+'</h3><small>'+e(r.market||'Source not specified')+' · Captured '+e(new Date(r.capturedAt).toLocaleString())+' by '+e(r.author)+'</small><p>'+e(r.verification||'User-supplied, not independently verified')+'. Prices and ratings can change.</p>'+(link(r.url)?'<p><a target="_blank" rel="noopener noreferrer" href="'+e(link(r.url))+'">Open source ↗</a></p>':'')+(r.price!==null&&r.price!==undefined?'<p>Observed price: '+money(r.price)+'</p>':'')+(r.rating!==null&&r.rating!==undefined?'<p>Source rating: '+e(r.rating)+'/5'+(r.reviewCount!==null&&r.reviewCount!==undefined?' · '+e(r.reviewCount)+' reviews':'')+'</p>':'')+(r.sourceAuthor?'<p>Source author: '+e(r.sourceAuthor)+'</p>':'')+(r.comment?'<p>'+e(r.comment).replace(/\n/g,'<br>')+'</p>':'')+(evidenceImageAllowed(r.picture)?'<img loading="lazy" alt="'+e(r.picture.name||r.title)+'" style="max-width:100%;max-height:260px" src="'+e(r.picture.data)+'">':'')+(link(r.photoUrl)?'<p><a target="_blank" rel="noopener noreferrer" href="'+e(link(r.photoUrl))+'">Open picture source ↗</a></p>':'')+(r.capture?'<details><summary>Captured source text</summary><pre>'+e(r.capture)+'</pre></details>':'')+notesMarkup(r.id)+'<div class="row">'+button('evidence-comment','Discuss evidence',r.id)+(editable()?button('evidence-remove',r.archived?'Restore':'Archive',r.id):'')+'</div></section>').join('')+'</details>';
}
function evidenceRemove(id){const row=(current().evidence??[]).find(x=>x.id===id);if(!row||!editable())return;apply((row.archived?'Restored':'Archived')+' evidence: '+row.title,s=>{s.evidence.find(x=>x.id===id).archived=!row.archived})}
document.addEventListener('click',ev=>{const b=ev.target.closest('button');if(!b)return;switch(b.dataset.act){
 case 'evidence-extract':{const data=evidenceExtract(form.elements.capture.value);for(const key of ['title','url','price','rating','reviewCount'])if(!form.elements[key].value&&data[key]!==undefined)form.elements[key].value=data[key];if(!form.elements.market.value&&data.url)form.elements.market.value=new URL(data.url).hostname;break}
 case 'evidence-comment':comment([b.dataset.id]);break;
 case 'evidence-remove':evidenceRemove(b.dataset.id);break;
}});
