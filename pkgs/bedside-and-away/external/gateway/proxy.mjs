import http from 'node:http';
import crypto from 'node:crypto';
import fs from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import {assets as assetPaths,assetType} from '../../scripts/assets.mjs';
const LOGIN=/^[^\s@]+@[^\s@]+$/;
export function makeGateway({workerOrigin,origin,key,allowedLogins,fetcher=fetch,assets}){
 const upstream=new URL(workerOrigin), site=new URL(origin);
 if(upstream.protocol!=='https:'||upstream.username||upstream.password||upstream.href!==upstream.origin+'/'||site.protocol!=='https:'||site.href!==site.origin+'/'||!key||key.length<32||!allowedLogins.size)throw Error('Set valid HTTPS origins, a signing key and explicit allowed logins');
 return http.createServer(async(req,res)=>{
   const reject=(code,message)=>{res.writeHead(code,{'Content-Type':'text/plain; charset=utf-8','Cache-Control':'no-store'});res.end(message)};
   try{
     const rawLogin=req.headers['tailscale-user-login'];const login=typeof rawLogin==='string'?rawLogin.trim().toLowerCase():'';
     if(req.socket.remoteAddress!=='127.0.0.1'||!LOGIN.test(login)||!allowedLogins.has(login))return reject(403,'A permitted Tailscale user is required.');
     if(!req.url?.startsWith('/')||req.url.startsWith('//')||req.url.includes('\\'))return reject(400,'Bad request path.');
     const incoming=new URL(req.url,site.origin);
     if(incoming.origin!==site.origin||!Object.hasOwn(assetPaths,incoming.pathname)&&!incoming.pathname.startsWith('/api/'))return reject(404,'Not found.');
     const method=req.method;
     if(!['GET','HEAD','POST'].includes(method)||method==='POST'&&!incoming.pathname.startsWith('/api/'))return reject(405,'Method not allowed.');
     if(method==='POST'&&(req.headers.origin!==site.origin||req.headers['sec-fetch-site']==='cross-site'||!/^application\/json(?:;|$)/i.test(req.headers['content-type']||'')))return reject(403,'Same-origin JSON required.');
     if(!incoming.pathname.startsWith('/api/')){
       if(!assets)return reject(404,'Not found.');
       const path=assetPaths[incoming.pathname];
       const type=assetType(path);
       const asset=await fs.readFile(new URL(path,assets));
       res.writeHead(200,{'Content-Type':type,'Cache-Control':'no-cache','X-Content-Type-Options':'nosniff','Referrer-Policy':'no-referrer','Content-Security-Policy':"default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; base-uri 'none'; form-action 'self'; object-src 'none'; frame-ancestors 'none'"});
       res.end(method==='HEAD'?undefined:asset);return;
     }
     const chunks=[];let length=0;
     for await(const x of req){length+=x.length;if(length>1700000){reject(413,'Request too large.');return}chunks.push(x)}
     const body=Buffer.concat(chunks);
     if(method!=='POST'&&body.length)return reject(400,'Unexpected body.');
     const time=String(Math.floor(Date.now()/1000)),nonce=crypto.randomUUID();
     const actor=Buffer.from(JSON.stringify({id:'ts_'+crypto.createHash('sha256').update(login).digest('hex'),email:login})).toString('base64url');
     const path=incoming.pathname+incoming.search;
     const hash=crypto.createHash('sha256').update(body).digest('hex');
     const signed=['bedaway-gateway-v1',time,nonce,method,path,actor,site.origin,hash].join('\n');
     const sig=crypto.createHmac('sha256',key).update(signed).digest('hex');
     const headers={'x-bedaway-time':time,'x-bedaway-nonce':nonce,'x-bedaway-actor':actor,'x-bedaway-origin':site.origin,'x-bedaway-signature':sig};
     if(method==='POST')headers['content-type']='application/json';
     const r=await fetcher(new URL(path,upstream.origin),{method,headers,body:method==='POST'?body:undefined,redirect:'error',signal:AbortSignal.timeout(25000)});
     const outgoing={'Cache-Control':'no-store','X-Content-Type-Options':'nosniff','Referrer-Policy':'no-referrer'};
     for(const h of ['content-type','content-security-policy','permissions-policy','content-disposition'])if(r.headers.has(h))outgoing[h]=r.headers.get(h);
     res.writeHead(r.status,outgoing);res.end(Buffer.from(await r.arrayBuffer()));
   }catch{if(!res.headersSent)reject(502,'The private backend could not be reached.');else res.end()}
 });
}
if(process.argv[1]&&pathToFileURL(process.argv[1]).href===import.meta.url){
 const p=Number(process.env.GATEWAY_PORT||4178);
 if(!Number.isInteger(p)||p<1024||p>65535)throw Error('Invalid loopback port');
 makeGateway({workerOrigin:process.env.WORKER_ORIGIN,origin:process.env.GATEWAY_ORIGIN,key:process.env.GATEWAY_HMAC_KEY,assets:new URL('../../public/',import.meta.url),allowedLogins:new Set((process.env.TAILSCALE_ALLOWED_LOGINS??'').split(',').map(x=>x.trim().toLowerCase()).filter(Boolean))}).listen(p,'127.0.0.1',()=>console.log('Bedside gateway listening on loopback port '+p));
}
