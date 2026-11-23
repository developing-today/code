import test from 'node:test';
import assert from 'node:assert/strict';
import {clearAccessKeyCacheForTests, verifyAccessServiceToken, BRIDGE_HOST} from './access-auth.mjs';
import worker from './worker.mjs';
import {b64url, operations} from './protocol.mjs';

const now = 1_800_000_000_000;
const issuer = 'https://fleet-team.cloudflareaccess.com';
const audience = 'bridge-access-audience-1234567890';
const clientId = 'amd-client-123456';
const env = {ACCESS_ISSUER:issuer,ACCESS_AUDIENCE:audience,BRIDGE_HOST};
const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
async function fixture() {
  const pair = await crypto.subtle.generateKey({name:'RSASSA-PKCS1-v1_5',modulusLength:2048,publicExponent:new Uint8Array([1,0,1]),hash:'SHA-256'},true,['sign','verify']);
  const jwk = await crypto.subtle.exportKey('jwk',pair.publicKey);
  Object.assign(jwk,{kid:'key-current',alg:'RS256',use:'sig'});
  const fetchImpl = async url => {
    assert.equal(url,`${issuer}/cdn-cgi/access/certs`);
    return Response.json({keys:[jwk]});
  };
  const token = async (changes={}, signingPair=pair, baseNow=now) => {
    const head = encode({alg:'RS256',typ:'JWT',kid:'key-current'});
    const baseSeconds=Math.floor(baseNow/1000);
    const claims = encode({type:'app',aud:[audience],exp:baseSeconds+300,iat:baseSeconds-1,iss:issuer,common_name:clientId,sub:'',...changes});
    const input = `${head}.${claims}`;
    const signature = Buffer.from(await crypto.subtle.sign('RSASSA-PKCS1-v1_5',signingPair.privateKey,new TextEncoder().encode(input))).toString('base64url');
    return `${input}.${signature}`;
  };
  const request = async (claims={}, {host=BRIDGE_HOST, fetch=fetchImpl, id=clientId, tokenOverride, signingPair, nowMs=now, access=audience, includeClientId=true}={}) => {
    const jwt = tokenOverride ?? await token(claims,signingPair,nowMs);
    const headers = {'cf-access-client-secret':'synthetic-secret-not-live','cf-access-jwt-assertion':jwt};
    if(includeClientId) headers['cf-access-client-id']=id;
    const req = new Request(`https://${host}/v1/poll`,{method:'POST',headers});
    return verifyAccessServiceToken(req,env,{access:access===null?undefined:{aud:access}},{fetchImpl:fetch,nowMs});
  };
  return {pair,fetchImpl,token,request};
}

test('Access service token validates signature, issuer, claim and context audiences, common_name, times and exact hostname',async()=>{
  clearAccessKeyCacheForTests();
  const f=await fixture();
  assert.equal(await f.request(),clientId);
  assert.equal(await f.request({}, {includeClientId:false}),clientId);
  // The edge-provided header is not authoritative; only the signed common_name selects identity.
  assert.equal(await f.request({}, {id:'different-untrusted-client'}),clientId);
  assert.equal(await f.request({exp:now/1000+3600,iat:now/1000-1800}),clientId);
  for(const claims of [
    {iss:'https://attacker.cloudflareaccess.com'},
    {aud:['another-audience']},
    {exp:now/1000-1,iat:now/1000-100},
    {iat:now/1000+31},
    {common_name:'other-client'},
    {sub:'nonempty'},
    {type:'org'},
  ]) await assert.rejects(f.request(claims),/access-denied/);
  await assert.rejects(f.request({}, {access:'wrong-access-context-audience'}),/access-denied/);
  await assert.rejects(f.request({}, {access:null}),/access-denied/);
  await assert.rejects(f.request({}, {host:'other.security.cab'}),/access-denied/);
});

test('forged signatures, unsupported algorithms, unknown keys and unavailable/stale JWKS deny',async()=>{
  clearAccessKeyCacheForTests();
  const f=await fixture();
  const good=await f.token();
  const [head,payload,sig]=good.split('.');
  const forged=`${head}.${payload}.${Buffer.alloc(256,7).toString('base64url')}`;
  await assert.rejects(f.request({}, {tokenOverride:forged}),/access-denied/);
  const badAlg=`${encode({alg:'HS256',typ:'JWT',kid:'key-current'})}.${payload}.${sig}`;
  await assert.rejects(f.request({}, {tokenOverride:badAlg}),/access-denied/);
  const unknown=`${encode({alg:'RS256',typ:'JWT',kid:'revoked-key'})}.${payload}.${sig}`;
  await assert.rejects(f.request({}, {tokenOverride:unknown}),/access-denied/);
  clearAccessKeyCacheForTests();
  await assert.rejects(f.request({}, {fetch:async()=>new Response('redirect',{status:302,headers:{location:'https://attacker.invalid'}})}),/access-denied/);
  clearAccessKeyCacheForTests();
  await f.request(); // populate bounded key cache
  clearAccessKeyCacheForTests();
  await assert.rejects(f.request({}, {fetch:async()=>new Response('unavailable',{status:503})}),/access-denied/);
});

test('deployed fetch binds signed Access identity to the same Ed25519 principal and strips edge credentials before DO',async()=>{
  clearAccessKeyCacheForTests();
  const f=await fixture();
  const ed=await crypto.subtle.generateKey('Ed25519',true,['sign','verify']);
  const edCaller=await crypto.subtle.generateKey('Ed25519',true,['sign','verify']);
  const principal={id:'amd-device',role:'device',device:'amd',publicKey:b64url(await crypto.subtle.exportKey('raw',ed.publicKey)),
    accessClientId:clientId,scopes:[...operations],issuedAt:Date.now()-1000,expiresAt:Date.now()+600000,revoked:false};
  const caller={...principal,id:'aster-caller',role:'caller',publicKey:b64url(await crypto.subtle.exportKey('raw',edCaller.publicKey)),accessClientId:'aster-client-123456'};
  let received;
  const workerEnv={...env,ENABLED:'true',BRIDGE_REGISTRY:JSON.stringify({audience:'bridge-signing-audience',principals:[principal,caller]}),
    BRIDGE:{idFromName:id=>id,get:()=>({fetch:async req=>{received=req;return Response.json({ok:true});}})}};
  const currentNow=Date.now();
  const jwt=await f.token({},undefined,currentNow);
  const req=new Request(`https://${BRIDGE_HOST}/v1/poll`,{method:'POST',body:'{}',headers:{
    'content-type':'application/json','x-bridge-principal':'amd-device','x-bridge-timestamp':String(currentNow),'x-bridge-nonce':'nonce-test',
    'x-bridge-signature':'synthetic-signature','cf-access-client-id':clientId,'cf-access-client-secret':'synthetic-secret',
    'cf-access-jwt-assertion':jwt,'cf-access-authenticated-user-email':'forged@example.invalid',cookie:'CF_Authorization=synthetic',authorization:'Bearer synthetic',
  }});
  assert.equal(await verifyAccessServiceToken(req,workerEnv,{access:{aud:audience}},{nowMs:currentNow,fetchImpl:f.fetchImpl}),clientId);
  const accessContext={access:{aud:audience}};
  const result=await worker.fetch(req,workerEnv,accessContext);
  assert.equal(result.status,200);
  assert.equal(received.headers.get('cf-access-jwt-assertion'),null);
  assert.equal(received.headers.get('cf-access-client-secret'),null);
  assert.equal(received.headers.get('cf-access-authenticated-user-email'),null);
  assert.equal(received.headers.get('cookie'),null);
  assert.equal(received.headers.get('authorization'),null);
  assert.equal(received.headers.get('x-bridge-signature'),'synthetic-signature');
  received=null;
  const forged=new Request(`https://${BRIDGE_HOST}/v1/poll`,{method:'POST',body:'{}',headers:{'cf-access-client-id':'other-client','cf-access-jwt-assertion':jwt}});
  assert.equal((await worker.fetch(forged,workerEnv,accessContext)).status,401);
  assert.equal(received,null);
  const crossRole=new Request(`https://${BRIDGE_HOST}/v1/poll`,{method:'POST',body:'{}',headers:{'x-bridge-principal':'aster-caller','cf-access-jwt-assertion':jwt}});
  assert.equal((await worker.fetch(crossRole,workerEnv,accessContext)).status,401);
  assert.equal(received,null);
  const unknownJwt=await f.token({common_name:'unknown-client-123'},undefined,currentNow);
  assert.equal(await verifyAccessServiceToken(new Request(`https://${BRIDGE_HOST}/v1/poll`,{method:'POST',headers:{'cf-access-jwt-assertion':unknownJwt}}),workerEnv,accessContext,{nowMs:currentNow,fetchImpl:f.fetchImpl}),'unknown-client-123');
  const unknownPrincipal=new Request(`https://${BRIDGE_HOST}/v1/poll`,{method:'POST',body:'{}',headers:{'x-bridge-principal':'unknown-principal','cf-access-jwt-assertion':unknownJwt}});
  assert.equal((await worker.fetch(unknownPrincipal,workerEnv,accessContext)).status,401);
  assert.equal(received,null);
  principal.revoked=true;
  workerEnv.BRIDGE_REGISTRY=JSON.stringify({audience:'bridge-signing-audience',principals:[principal,caller]});
  assert.equal((await worker.fetch(req,workerEnv,accessContext)).status,401);
});

test('default fetch rejects nonexact routes and upgrades before JWKS or Durable Object access',async()=>{
  clearAccessKeyCacheForTests();
  const f=await fixture();
  const ed=await crypto.subtle.generateKey('Ed25519',true,['sign','verify']);
  const edCaller=await crypto.subtle.generateKey('Ed25519',true,['sign','verify']);
  const principal={id:'amd-device',role:'device',device:'amd',publicKey:b64url(await crypto.subtle.exportKey('raw',ed.publicKey)),accessClientId:clientId,scopes:[...operations],issuedAt:Date.now()-1000,expiresAt:Date.now()+600000,revoked:false};
  const caller={...principal,id:'aster-caller',role:'caller',publicKey:b64url(await crypto.subtle.exportKey('raw',edCaller.publicKey)),accessClientId:'aster-client-123456'};
  let doCalls=0;
  const workerEnv={...env,ENABLED:'true',BRIDGE_REGISTRY:JSON.stringify({audience:'bridge-signing-audience',principals:[principal,caller]}),
    BRIDGE:{idFromName:id=>id,get:()=>({fetch:async()=>{doCalls++;return Response.json({ok:true});}})}};
  const jwt=await f.token({},undefined,Date.now());
  const ignoredFetch=async()=>{throw new Error('JWKS must not be fetched for rejected routes');};
  const invalidRoute=new Request(`https://${BRIDGE_HOST}/v1/unknown`,{method:'POST',headers:{'cf-access-jwt-assertion':jwt}});
  const upgrade=new Request(`https://${BRIDGE_HOST}/v1/poll`,{method:'POST',headers:{'cf-access-jwt-assertion':jwt,upgrade:'websocket'}});
  assert.equal((await worker.fetch(invalidRoute,workerEnv,{access:{aud:audience}})).status,404);
  assert.equal((await worker.fetch(upgrade,workerEnv,{access:{aud:audience}})).status,404);
  await assert.rejects(verifyAccessServiceToken(invalidRoute,workerEnv,{access:{aud:audience}},{fetchImpl:ignoredFetch}),/access-denied/);
  await assert.rejects(verifyAccessServiceToken(upgrade,workerEnv,{access:{aud:audience}},{fetchImpl:ignoredFetch}),/access-denied/);
  assert.equal(doCalls,0);
});
