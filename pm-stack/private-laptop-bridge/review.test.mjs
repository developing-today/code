import test from 'node:test';
import assert from 'node:assert/strict';
import {createClient,BridgeDenied} from './client.mjs';
import {createCaller} from './caller.mjs';
import worker,{PrivateLaptopBridge} from './worker.mjs';
import {b64url,boundedText} from './protocol.mjs';

const NOW=1800000000000;
const keys={caller:await crypto.subtle.generateKey('Ed25519',true,['sign','verify']),device:await crypto.subtle.generateKey('Ed25519',true,['sign','verify'])};
const origin='https://laptop-bridge-staging.security.cab',audience='review-test';
const result={ok:true,data:{queueBroker:'up'}};
class Storage {
  value;
  async transaction(fn){let next=structuredClone(this.value);const out=await fn({get:async()=>structuredClone(next),put:async(_,v)=>{next=structuredClone(v);},setAlarm:async()=>{}});this.value=next;return out;}
}
async function harness(t,offset=0) {
  let clock=NOW;
  t.mock.method(Date,'now',()=>clock);
  const principals=[];
  for(const [id,role] of [['caller','caller'],['device','device']])principals.push({id,role,device:'amd',publicKey:b64url(await crypto.subtle.exportKey('raw',keys[id].publicKey)),accessClientId:`synthetic-${id}-client`,scopes:['laptop.health'],issuedAt:NOW-60000,expiresAt:NOW+1800000,revoked:false});
  const env={ENABLED:'true',BRIDGE_REGISTRY:JSON.stringify({audience,principals})};
  const object=new PrivateLaptopBridge({storage:new Storage()},env);
  env.BRIDGE={idFromName:x=>x,get:()=>({fetch:req=>object.fetch(req)})};
  const dispatch=req=>object.fetch(req);
  const config={origin,audience,now:()=>clock-offset,fetch:dispatch,accessClientId:'synthetic-client-id',accessClientSecret:'synthetic-access-secret-that-is-never-live'};
  const caller=createCaller({...config,principal:'caller',device:'amd',privateKey:keys.caller.privateKey});
  const deviceConfig={...config,principal:'device',privateKey:keys.device.privateKey,getuid:()=>1000,execute:async()=>result};
  return {caller,deviceConfig,dispatch,advance:ms=>{clock+=ms;},localNow:()=>clock-offset};
}

for(const offset of [-29000,-500,0,500,29000])test(`clock oracle: authenticated ${offset}ms offset preserves conservative caller/device lifetimes`,async t=>{
  const h=await harness(t,offset);
  const handle=await h.caller.laptopHealth();
  assert.equal(handle.expiresAt,h.localNow()+60000);
  const client=createClient(h.deviceConfig);
  assert.equal(await client.step(),true);
  assert.deepEqual((await h.caller.read(handle)).result,result);
});

test('deadline oracle: delayed poll response consumes the lease before any adapter executes',async t=>{
  const h=await harness(t,29000);await h.caller.laptopHealth();let executed=0;
  const client=createClient({...h.deviceConfig,execute:async()=>{executed++;return result;},fetch:async request=>{const response=await h.dispatch(request);h.advance(16000);return response;}});
  await assert.rejects(client.step(),/expired-job/);assert.equal(executed,0);
});

for(const offset of [-29000,29000])test(`deadline oracle: actual lease expiration blocks completion with ${offset}ms skew`,async t=>{
  const h=await harness(t,offset);await h.caller.laptopHealth();let completions=0;
  const client=createClient({...h.deviceConfig,execute:async()=>{h.advance(15000);return result;},fetch:async request=>{if(new URL(request.url).pathname==='/v1/result')completions++;return h.dispatch(request);}});
  await assert.rejects(client.step(),/expired-job/);assert.equal(completions,0);
});

test('deadline oracle: delayed call acknowledgement is never exposed as a fresh handle',async t=>{
  const h=await harness(t,500);
  const caller=createCaller({origin,audience,principal:'caller',device:'amd',privateKey:keys.caller.privateKey,accessClientId:'synthetic-client-id',accessClientSecret:'synthetic-access-secret-that-is-never-live',now:h.localNow,fetch:async request=>{const response=await h.dispatch(request);h.advance(60000);return response;}});
  await assert.rejects(caller.laptopHealth(),{code:'submit-outcome-unknown'});
});

test('schema oracle: inconsistent server deadlines and excessive skew are rejected before execution',async()=>{
  const base={id:'job',lease:'lease',operation:'laptop.health',args:{},serverNow:NOW,expiresAt:NOW+60000,leaseExpiresAt:NOW+15000};
  const bad=[{...base,serverNow:NOW+30001},{...base,serverNow:NOW-30001},{...base,expiresAt:NOW+60001},{...base,leaseExpiresAt:NOW+15001},{...base,leaseExpiresAt:NOW},{...base,expiresAt:NOW+1000,leaseExpiresAt:NOW+2000},{...base,serverNow:undefined}];
  let executed=0;
  for(const job of bad){const client=createClient({origin,audience,principal:'device',privateKey:keys.device.privateKey,accessClientId:'synthetic-client-id',accessClientSecret:'synthetic-access-secret-that-is-never-live',now:()=>NOW,getuid:()=>1000,execute:async()=>{executed++;return result;},fetch:async()=>Response.json({job})});await assert.rejects(client.step(),/invalid-job/);}
  assert.equal(executed,0);
  for(const body of [{id:'job',serverNow:NOW+30001,expiresAt:NOW+60000},{id:'job',serverNow:NOW,expiresAt:NOW+60001},{id:'job',expiresAt:NOW+60000}]){
    const caller=createCaller({origin,audience,principal:'caller',device:'amd',privateKey:keys.caller.privateKey,accessClientId:'synthetic-client-id',accessClientSecret:'synthetic-access-secret-that-is-never-live',now:()=>NOW,fetch:async()=>Response.json(body,{status:202})});
    await assert.rejects(caller.laptopHealth(),{code:'submit-outcome-unknown'});
  }
});

test('cleanup oracle: denial is immediate even when stream cancellation stalls or rejects',async()=>{
  for(const status of [401,403,410,503])for(const cancel of [()=>new Promise(()=>{}),()=>Promise.reject(new Error('private cancellation detail'))]){
    const client=createClient({origin,audience,principal:'device',privateKey:keys.device.privateKey,accessClientId:'synthetic-client-id',accessClientSecret:'synthetic-access-secret-that-is-never-live',getuid:()=>1000,requestTimeoutMs:5,fetch:async()=>new Response(new ReadableStream({cancel}),{status})});
    let timer;
    try {
      const got=await Promise.race([client.step().then(()=>null,error=>error),new Promise(resolve=>{timer=setTimeout(()=>resolve('stalled'),100);})]);
      assert.notEqual(got,'stalled');
      if(status===503)assert.equal(got.message,'bridge-unavailable');else assert.ok(got instanceof BridgeDenied);
    }finally{clearTimeout(timer);}
  }
});

test('cleanup oracle: Worker body timeout does not await hostile cancellation',async()=>{
  await assert.rejects(boundedText(new ReadableStream({cancel:()=>new Promise(()=>{})}),8192,5),/timeout/);
});
