import {api} from '../worker/api.mjs';
import {createTurso} from './turso.mjs';
import {verifyGateway} from './auth.mjs';
const denied=()=>new Response(JSON.stringify({error:'Authenticated gateway required'}),{status:401,headers:{'content-type':'application/json','cache-control':'no-store'}});
export default {async fetch(request,env){
 let auth;try{auth=await verifyGateway(request,env)}catch{return denied()}
 if(!['GET','POST'].includes(auth.method)||!auth.path.startsWith('/api/'))return new Response('Not found',{status:404});
 let stage='adapter';
 try{
  const db=createTurso({url:env.TURSO_URL,token:env.TURSO_AUTH_TOKEN});
  // Persistent one-use nonces close replay across Worker isolates. Missing schema fails closed.
  stage='nonce';
  const rows=await db.batch([
   db.prepare('DELETE FROM gateway_nonces WHERE expires_at < ?').bind(Math.floor(Date.now()/1000)),
   db.prepare('INSERT OR IGNORE INTO gateway_nonces(nonce,expires_at) VALUES(?,?)').bind(auth.nonce,auth.expires)
  ]);
  if(rows[1].meta.changes!==1)return denied();
  // Never forward browser/provider identity or origin headers. Rebuild trusted headers.
  stage='request';
  const headers=new Headers({'oai-authenticated-user-id':auth.actor.id,'oai-authenticated-user-email':auth.actor.email});
  if(auth.method==='POST'){headers.set('origin',env.GATEWAY_ORIGIN);headers.set('content-type','application/json')}
  const upstream=new Request(new URL(auth.path,env.GATEWAY_ORIGIN),{method:auth.method,headers,...(auth.method==='POST'?{body:auth.bytes}: {})});
  stage='api';
  return await api(upstream,{DB:db});
 }catch(err){console.warn('bedaway backend failure',err?.phase??err?.name??'unknown');return new Response(JSON.stringify({error:'Backend unavailable. Do not repeat uncertain writes without checking their outcome.',phase:[stage,err?.phase??err?.name??'unknown',/illegal invocation/i.test(err?.message)?'invocation':/invalid url/i.test(err?.message)?'url':/network|fetch failed|connection|dns/i.test(err?.message)?'network':/undefined|null|iterable/i.test(err?.message)?'shape':/header|body/i.test(err?.message)?'request':'other'].join(':')}),{status:503,headers:{'content-type':'application/json','cache-control':'no-store'}})}
}};
