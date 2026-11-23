// Real Workers RPC + SQLite Durable Object tests. The Gateway metadata provider
// is synthetic; these tests do not prove Cloudflare Gateway/WARP/DNS enrollment.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Miniflare } from 'miniflare';
import { b64url, operations, signRequest } from './protocol.mjs';
import bridgeWorker from './worker.mjs';
import {gatewayFetch} from './gateway-fetch.mjs';
import {BRIDGE_HOST,clearAccessKeyCacheForTests,verifyAccessServiceToken} from './access-auth.mjs';
const here = dirname(fileURLToPath(import.meta.url));
const host = 'private-bridge-staging.invalid';
const account = 'a'.repeat(32), policy = '11111111-1111-1111-1111-111111111111', posture = '22222222-2222-2222-2222-222222222222';
const email = 'synthetic-owner@example.invalid';

test('configured gateway direct-fetch adapter preserves the Cloudflare Access context',async()=>{
  const entrypoint=await readFile(new URL('./gateway.mjs',import.meta.url),'utf8');
  assert.match(entrypoint,/fetch\(request, env, ctx\)\s*\{\s*return gatewayFetch\(bridge, request, env, ctx\);/);
  clearAccessKeyCacheForTests();
  const issuer='https://gateway-test.cloudflareaccess.com',audience='gateway-access-audience-1234567890';
  const clientId='gateway-caller-client',nowMs=Date.now(),seconds=Math.floor(nowMs/1000);
  const rsa=await crypto.subtle.generateKey({name:'RSASSA-PKCS1-v1_5',modulusLength:2048,publicExponent:new Uint8Array([1,0,1]),hash:'SHA-256'},true,['sign','verify']);
  const jwk=await crypto.subtle.exportKey('jwk',rsa.publicKey);Object.assign(jwk,{kid:'gateway-key',alg:'RS256',use:'sig'});
  const encode=value=>Buffer.from(JSON.stringify(value)).toString('base64url');
  const head=encode({alg:'RS256',typ:'JWT',kid:'gateway-key'});
  const claims=encode({type:'app',aud:[audience],exp:seconds+300,iat:seconds-1,iss:issuer,common_name:clientId,sub:''});
  const input=`${head}.${claims}`;
  const signature=Buffer.from(await crypto.subtle.sign('RSASSA-PKCS1-v1_5',rsa.privateKey,new TextEncoder().encode(input))).toString('base64url');
  const request=new Request(`https://${BRIDGE_HOST}/v1/read`,{method:'POST',headers:{'cf-access-jwt-assertion':`${input}.${signature}`,'x-bridge-principal':'caller'}});
  const ed=await crypto.subtle.generateKey('Ed25519',true,['sign','verify']);
  const principal={id:'caller',role:'caller',device:'amd',publicKey:b64url(await crypto.subtle.exportKey('raw',ed.publicKey)),accessClientId:clientId,scopes:[...operations],issuedAt:nowMs-1000,expiresAt:nowMs+600000,revoked:false};
  let received;
  const env={ENABLED:'true',BRIDGE_HOST,ACCESS_ISSUER:issuer,ACCESS_AUDIENCE:audience,BRIDGE_REGISTRY:JSON.stringify({audience:'bridge-signing-audience',principals:[principal,{...principal,id:'device',role:'device',accessClientId:'gateway-device-client',publicKey:b64url(await crypto.subtle.exportKey('raw',(await crypto.subtle.generateKey('Ed25519',true,['sign','verify'])).publicKey))}]}),BRIDGE:{idFromName:id=>id,get:()=>({fetch:async req=>{received=req;return Response.json({ok:true});}})}};
  const ctx={access:{aud:audience}};
  await verifyAccessServiceToken(request,env,ctx,{fetchImpl:async()=>Response.json({keys:[jwk]}),nowMs});
  const response=await gatewayFetch(bridgeWorker,request,env,ctx);
  assert.equal(response.status,200);
  assert.equal(received.headers.get('x-bridge-principal'),'caller');
  assert.equal(received.headers.get('cf-access-jwt-assertion'),null);
  assert.equal((await response.json()).ok,true);
});
const harnessSource = `
export default {
 async fetch(request, env) {
   const mode = request.headers.get('x-test-mode') || 'normal';
   const url = new URL(request.url);
   const context = {
     accountId: '${account}', policyId: '${policy}', host: '${host}', uri: request.url, sni: '${host}',
     identity: async () => {
       if (mode === 'identity-hang') return new Promise(() => {});
       if (mode === 'identity-error') throw new Error('synthetic identity failure');
       if (mode === 'identity-missing') return null;
       return {email: mode === 'identity-wrong' ? 'other@example.invalid' : '${email}'};
     },
     devicePosture: async () => {
       if (mode === 'posture-error') throw new Error('synthetic posture failure');
       return {checks:{passed: mode === 'posture-missing' ? [] : ['${posture}']}};
     }
   };
   if (mode === 'account-wrong') context.accountId = 'b'.repeat(32);
   if (mode === 'policy-wrong') context.policyId = '33333333-3333-3333-3333-333333333333';
   if (mode === 'context-host-wrong') context.host = 'other.invalid';
   if (mode === 'context-uri-wrong') context.uri = 'https://${host}/v1/poll';
   if (mode === 'context-sni-wrong') context.sni = 'other.invalid';
   if (mode === 'identity-header') { context.identity = async () => null; const headers = new Headers(request.headers); headers.set('cf-access-authenticated-user-email','${email}'); request = new Request(request, {headers}); }
   let nextCalls = 0;
   const response = await env.PRIVATE.handle(request, mode === 'context-missing' ? null : context, async () => {nextCalls++; throw new Error('next must never run');});
   const headers = new Headers(response.headers); headers.set('x-test-next-calls', String(nextCalls));
   return new Response(response.body, {status: response.status, headers});
 }
};`;
async function harness(t, overrides = {}) {
  const keys = {}, principals = [];
  for (const [id, role] of [['caller','caller'], ['device','device']]) {
    keys[id] = await crypto.subtle.generateKey('Ed25519', true, ['sign','verify']);
    principals.push({id, role, device:'amd', publicKey:b64url(await crypto.subtle.exportKey('raw', keys[id].publicKey)), accessClientId:`synthetic-${id}-client`,scopes:[...operations], issuedAt:Date.now()-1000, expiresAt:Date.now()+120000, revoked:false});
  }
  const mf = new Miniflare({host:'127.0.0.1', workers:[
    {name:'gateway-harness', modules:true, script:harnessSource, compatibilityDate:'2026-07-30', serviceBindings:{PRIVATE:{name:'private-bridge',entrypoint:'GatewayMiddleware'}}, outboundService:() => {throw new Error('outbound forbidden');}},
    {name:'private-bridge', modules:true, scriptPath:join(here,'gateway.mjs'), modulesRules:[{type:'ESModule',include:['**/*.mjs']}], compatibilityDate:'2026-07-30', bindings:{ENABLED:'true', BRIDGE_REGISTRY:JSON.stringify({audience:'gateway-offline',principals}), GATEWAY_HOST:host,GATEWAY_ACCOUNT_ID:account,GATEWAY_POLICY_ID:policy,GATEWAY_IDENTITY_EMAIL:email,GATEWAY_POSTURE_ID:posture,...overrides},durableObjects:{BRIDGE:{className:'PrivateLaptopBridge',useSQLite:true}},outboundService:() => {throw new Error('outbound forbidden');}}
  ]});
  await mf.ready;
  t.after(() => mf.dispose());
  async function send(id, path, body, mode = 'normal', url = `https://${host}${path}`, method = 'POST') {
    const request = await signRequest({origin:`https://${host}`,audience:'gateway-offline',principal:id,privateKey:keys[id].privateKey,path,body});
    request.headers.set('x-test-mode',mode);
    const response = await mf.dispatchFetch(url,{method,headers:Object.fromEntries(request.headers),...(method !== 'GET' ? {body:await request.text()} : {})});
    assert.equal(response.headers.get('x-test-next-calls'),'0');
    return {status:response.status,body:await response.json()};
  }
  return {mf,send,keys};
}

test('Gateway RPC contract and direct Access HTTP gate are independent; ordinary HTTP is denied without Access config',async t=>{
  const h=await harness(t);
  const submitted=await h.send('caller','/v1/call',{device:'amd',operation:'laptop.health',args:{}});
  assert.equal(submitted.status,202,JSON.stringify(submitted));
  const polled=await h.send('device','/v1/poll',{});assert.equal(polled.status,200);
  const {job}=polled.body;assert.equal(job.id,submitted.body.id);
  const result={ok:true,data:{queueBroker:'up'}};
  assert.equal((await h.send('device','/v1/result',{id:job.id,lease:job.lease,result})).status,200);
  assert.deepEqual((await h.send('caller','/v1/read',{id:job.id})).body,{status:'complete',result});
  const direct=await h.mf.getWorker('private-bridge');
  const signed=await signRequest({origin:`https://${host}`,audience:'gateway-offline',principal:'caller',privateKey:h.keys.caller.privateKey,path:'/v1/call',body:{device:'amd',operation:'laptop.health',args:{}}});
  const response=await direct.fetch(signed.url,{method:'POST',headers:Object.fromEntries(signed.headers),body:await signed.text()});
  assert.equal(response.status,401);assert.deepEqual(await response.json(),{error:'access-authentication'});
});

test('private Gateway RPC rejects route and trusted-context mismatches without next',async t=>{
  const h=await harness(t);
  for(const mode of ['account-wrong','policy-wrong','context-host-wrong','context-uri-wrong','context-sni-wrong','context-missing','identity-missing','identity-wrong','posture-missing','identity-header']) {
    assert.equal((await h.send('caller','/v1/call',{device:'amd',operation:'laptop.health',args:{}},mode)).status,403,mode);
  }
  for(const mode of ['identity-error','posture-error','identity-hang']) assert.equal((await h.send('caller','/v1/call',{},mode)).status,503,mode);
  for(const url of [`http://${host}/v1/call`,`https://other.invalid/v1/call`,`https://${host}:8443/v1/call`,`https://${host}/exec`,`https://${host}/v1/call?extra=1`]) assert.equal((await h.send('caller','/v1/call',{},'normal',url)).status,403,url);
  assert.equal((await h.send('caller','/v1/call',{},'normal',`https://${host}/v1/call`,'GET')).status,403);
  // Gateway authorization cannot replace the independent signed role check.
  assert.equal((await h.send('caller','/v1/poll',{})).status,403);
});

test('private Gateway deployment remains disabled or fails closed on incomplete identifiers',async t=>{
  const disabled=await harness(t,{ENABLED:'false'});
  assert.equal((await disabled.send('caller','/v1/call',{})).status,503);
  const incomplete=await harness(t,{GATEWAY_POLICY_ID:''});
  assert.equal((await incomplete.send('caller','/v1/call',{})).status,503);
});
