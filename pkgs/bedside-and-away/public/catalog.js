'use strict';
function catalogText(value){return String(value??'').normalize('NFC').replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F\u200B-\u200F\u202A-\u202E\u2060-\u206F\uFEFF]/gu,'').trim()}
function catalogCanonicalURL(value){
 try{const u=new URL(String(value));if(!['http:','https:'].includes(u.protocol)||u.username||u.password)return '';
 if([...u.searchParams.keys()].some(k=>/^(?:access[_-]?token|refresh[_-]?token|id[_-]?token|token|auth|authorization|password|passwd|secret|api[_-]?key|key|signature|sig|credential|x-amz-.+|x-goog-.+)$/i.test(k)))return '';
 u.hash='';
 const amazon=/^(?:www\.)?amazon\.(?:com|ca|co\.uk|de|fr|it|es|co\.jp|com\.au)$/.test(u.hostname.toLowerCase());
 const asin=amazon&&u.pathname.match(/\/(?:dp|gp\/product)\/([A-Z0-9]{10})(?:\/|$)/i)?.[1];
 if(asin)return `https://${u.hostname.toLowerCase().replace(/^www\./,'')}/dp/${asin.toUpperCase()}`;
 for(const k of [...u.searchParams.keys()])if(/^(?:utm_.+|ref|tag|fbclid|gclid)$/i.test(k))u.searchParams.delete(k);
 u.searchParams.sort();return u.href;
 }catch{return ''}
}
function catalogIdentity(item){const url=catalogCanonicalURL(item.url);if(!url)return '';const variant=catalogText(item.variantSku||item.sku||'');return url+(variant?'#sku='+encodeURIComponent(variant):'')}
function catalogDedupe(items){
 const seen=new Map(),unique=[],duplicates=[],possible=[];
 for(const item of items){const key=catalogIdentity(item);if(key&&seen.has(key)){duplicates.push({item,existing:seen.get(key),reason:'Exact canonical URL and explicit SKU'});continue}if(key)seen.set(key,item);unique.push(item)}
 // A matching title is a suggestion, never automatic equivalence.
 const names=new Map();for(const item of unique){const key=catalogText(item.name||item.title).toLocaleLowerCase();if(!key)continue;if(names.has(key))possible.push({item,existing:names.get(key),reason:'Same title; variants may differ'});else names.set(key,item)}
 return {unique,duplicates,possible};
}
function catalogNormalizeMetadata(input){
 if(typeof input==='string'){try{input=JSON.parse(input)}catch{/* A plain string is preserved, not reinterpreted. */}}
 const entries=Array.isArray(input)?input.map((v,i)=>Array.isArray(v)&&v.length===2?v:[String(i),v]):input&&typeof input==='object'?Object.entries(input):[['value',input??null]];
 return entries.map(([key,value])=>({key:catalogText(key),value:typeof value==='string'?catalogText(value):value,type:value===null?'null':Array.isArray(value)?'array':typeof value}));
}
function catalogParsePaste(text,{capturedAt=new Date().toISOString(),source='Pasted by user'}={}){
 if(typeof text!=='string'||text.length>100000)throw Error('Paste at most 100,000 characters.');
 if(/-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----|\b(?:authorization:\s*bearer|session_token\s*[:=]|password\s*[:=])/i.test(text))throw Error('Remove credentials before importing.');
 let rows;try{const parsed=JSON.parse(text);rows=Array.isArray(parsed)?parsed:Array.isArray(parsed.items)?parsed.items:[parsed]}catch{rows=text.split(/\r?\n/).map(x=>x.trim()).filter(Boolean)}
 if(rows.length>500)throw Error('Import at most 500 items at a time.');
 const items=[],warnings=[];
 for(const [index,row] of rows.entries()){
  const obj=typeof row==='string'?{name:row}:row;if(!obj||typeof obj!=='object'||Array.isArray(obj)){warnings.push({index,message:'Not an item object or line'});continue}
  const rawURL=obj.url||String(obj.name||obj.title||'').match(/https?:\/\/[^\s<>"']+/)?.[0]||'';
  const url=catalogCanonicalURL(rawURL),name=catalogText(obj.name||obj.title||rawURL).replace(rawURL,'').trim()||url;
  if(rawURL&&!url){warnings.push({index,message:'Invalid or credential-bearing URL'});continue}if(!name){warnings.push({index,message:'Missing item name'});continue}
  items.push({name:name.slice(0,300),url,variantSku:catalogText(obj.variantSku||obj.sku),attributes:catalogNormalizeMetadata(obj.attributes||{}),provenance:{source:catalogText(source),capturedAt,verification:'User supplied; not independently verified'},importLine:index+1});
 }
 return {...catalogDedupe(items),warnings};
}
function catalogImportPlan(state,rows,newId){
 const staged=JSON.parse(JSON.stringify(state));const comparison=catalogDedupe([...staged.items,...rows]);const added=[];const ids=new Set(staged.items.map(x=>x.id));
 for(const row of comparison.unique){if(!rows.includes(row))continue;const id=newId();if(typeof id!=='string'||!id||ids.has(id))throw Error('ID allocation failed');ids.add(id);const item={...row,id,kind:'item',parent:'',tags:[],owner:'Shared',state:'Considering',hidden:false,archived:false,qty:1,price:null,description:''};staged.items.push(item);added.push(id)}
 state.items=staged.items;return {added,duplicates:comparison.duplicates,possible:comparison.possible};
}
