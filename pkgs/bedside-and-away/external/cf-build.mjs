// Contract: node external/cf-build.mjs (no arguments). Uses only non-secret config.
import fs from 'node:fs';
import path from 'node:path';
if(process.argv.length!==2)throw Error('Usage: node external/cf-build.mjs');
const url=new URL(process.env.TURSO_URL||'');
if(!['https:','libsql:'].includes(url.protocol)||url.username||url.password||url.search||url.hash)throw Error('Set a credential-free Turso URL');
const origin=new URL(process.env.GATEWAY_ORIGIN||'');
if(origin.protocol!=='https:'||origin.href!==origin.origin+'/'||!origin.hostname.endsWith('.ts.net'))throw Error('Set the HTTPS tailnet origin');
const name=process.env.BEDAWAY_WORKER_NAME||'bedside-and-away-backend';
if(!/^bedside-and-away-backend(?:-[a-z0-9-]+)?$/.test(name))throw Error('Set a Bedside & Away worker name');
const root=path.resolve('.cloudflare/output/v0'),w=path.join(root,'workers/default'),bundle=path.join(w,'bundle');
const save=(p,x)=>fs.writeFileSync(p,JSON.stringify(x,null,2)+'\n');
fs.mkdirSync(bundle,{recursive:true});
// cf beta.20 uploads multipart module names by basename. Keep the emitted
// graph flat and rewrite only imports to files in this exact graph.
const sources=new Map();
for(const directory of ['worker','external']){
 for(const entry of fs.readdirSync(new URL('../'+directory+'/',import.meta.url),{withFileTypes:true})){
  if(!entry.isFile()||!entry.name.endsWith('.mjs')||directory==='external'&&!['auth.mjs','turso.mjs','worker.mjs'].includes(entry.name))continue;
  const key=directory+'/'+entry.name;
  if(sources.has(entry.name))throw Error('Conflicting Worker module basename: '+entry.name);
  sources.set(entry.name,key);
 }
}
const modules={};
for(const [filename,key] of sources){
 const source=fs.readFileSync(new URL('../'+key,import.meta.url),'utf8').replace(/^(import\s+.+?\s+from\s+)(['"])([^'"]+)\2(\s*;)/gm,(_,head,quote,ref,end)=>{
  if(!ref.startsWith('.'))throw Error('Unexpected Worker module reference: '+ref);
  const resolved=path.posix.normalize(path.posix.join(path.posix.dirname(key),ref)),target=path.posix.basename(resolved);
  if(sources.get(target)!==resolved)throw Error('Missing Worker module: '+resolved);
  return head+quote+'./'+target+quote+end;
 });
 fs.writeFileSync(path.join(bundle,filename),source);
 modules[filename]={type:'esm'};
}
save(path.join(root,'config.json'),{buildContext:{isPreview:false}});
save(path.join(w,'worker.config.json'),{name,compatibilityDate:'2026-11-01',workersDev:true,previewUrls:false,env:{TURSO_URL:{type:'text',value:url.href},GATEWAY_ORIGIN:{type:'text',value:origin.origin},GATEWAY_HMAC_KEY:{type:'secret'},TURSO_AUTH_TOKEN:{type:'secret'}},manifest:{type:'complete',mainModule:'worker.mjs',modules}});
console.log('Generated Cloudflare prebuilt Worker; no secrets in build output.');
