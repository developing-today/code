// Local D1-style development bundle. Production uses external/cf-build.mjs.
import fs from 'node:fs';
import {assets as paths,assetType} from './assets.mjs';
const assets=Object.fromEntries(Object.entries(paths).map(([p,f])=>[p,[assetType(f),fs.readFileSync('public/'+f,'utf8')]]));
fs.mkdirSync('dist/server',{recursive:true});
fs.cpSync('worker','dist/server/worker',{recursive:true,filter:p=>!p.endsWith('.md')});
fs.writeFileSync('dist/server/index.js',"import {api} from './worker/api.mjs';\nconst assets="+JSON.stringify(assets)+';\n'+`export default {async fetch(req,env){const p=new URL(req.url).pathname;if(p.startsWith('/api/'))return api(req,env);if(!Object.hasOwn(assets,p))return new Response('Not found',{status:404});if(!['GET','HEAD'].includes(req.method))return new Response('Method not allowed',{status:405});const [type,body]=assets[p];return new Response(req.method==='HEAD'?null:body,{headers:{'Content-Type':type,'Cache-Control':'no-cache','Referrer-Policy':'no-referrer','X-Content-Type-Options':'nosniff','Content-Security-Policy':"default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; base-uri 'none'; form-action 'self'; object-src 'none'"}})}};\n`);
