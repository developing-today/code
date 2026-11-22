import test from 'node:test';
import assert from 'node:assert/strict';
import {Miniflare} from 'miniflare';
import {fileURLToPath} from 'node:url';
const host='private-bridge-smoke.invalid', account='a'.repeat(32), policy='11111111-1111-1111-1111-111111111111';
async function harness(t, overrides={}) {
  const mf=new Miniflare({host:'127.0.0.1',workers:[
    {name:'harness',modules:true,compatibilityDate:'2026-07-30',serviceBindings:{SMOKE:{name:'smoke',entrypoint:'GatewayMiddleware'}},script:`export default {async fetch(r,e){const c={accountId:'${account}',policyId:'${policy}',host:'${host}',uri:r.url,sni:'${host}'}; const mode=r.headers.get('x-mode'); if(mode==='upgrade') {const h=new Headers(r.headers);h.set('upgrade','websocket');r=new Request(r,{headers:h});} if(mode==='missing') return e.SMOKE.handle(r,null); if(mode==='wrong') c.policyId='wrong'; if(mode==='uri') c.uri+='?x'; return e.SMOKE.handle(r,c,()=>{throw Error('no next')});}}`},
    {name:'smoke',modules:true,compatibilityDate:'2026-07-30',scriptPath:fileURLToPath(new URL('./smoke.mjs',import.meta.url)),bindings:{ENABLED:'true',GATEWAY_HOST:host,GATEWAY_ACCOUNT_ID:account,GATEWAY_POLICY_ID:policy,START_MS:String(Date.now()-1000),END_MS:String(Date.now()+60000),...overrides},outboundService:()=>{throw Error('no outbound');}}
  ]}); await mf.ready; t.after(()=>mf.dispose()); return mf;
}
test('stateless smoke succeeds without keys, registry, identity getters, storage or upstream',async t=>{
 const mf=await harness(t); const r=await mf.dispatchFetch(`https://${host}/smoke/v1`); assert.equal(r.status,200);assert.equal(await r.text(),'private-bridge-smoke-v1\n');assert.equal(r.headers.get('cache-control'),'no-store');
 const direct=await mf.getWorker('smoke'); assert.equal((await direct.fetch(`https://${host}/smoke/v1`)).status,403);
});
test('smoke denies untrusted routes, credentials and context',async t=>{
 const mf=await harness(t);
 for(const [url,options] of [
  [`http://${host}/smoke/v1`,{}],[`https://${host}/smoke/v1?x`,{}],[`https://${host}/v1/call`,{}],
  ...['missing','wrong','uri','upgrade'].map(mode=>[`https://${host}/smoke/v1`,{headers:{'x-mode':mode}}]),
  ...['authorization','proxy-authorization','cookie','x-bridge-signature'].map(h=>[`https://${host}/smoke/v1`,{headers:{[h]:'synthetic'}}]),
  [`https://${host}/smoke/v1`,{method:'POST',body:'synthetic'}]
 ]) assert.equal((await mf.dispatchFetch(url,options)).status,403);
});
test('smoke requires explicit configuration and at most ten minute active window',async t=>{
 for(const override of [{ENABLED:'false'},{GATEWAY_ACCOUNT_ID:''},{GATEWAY_POLICY_ID:''},{START_MS:''},{END_MS:String(Date.now()-1)},{START_MS:String(Date.now()+60000),END_MS:String(Date.now()+120000)},{START_MS:String(Date.now()-1000),END_MS:String(Date.now()+600001)}]) {
  const mf=await harness(t,override); assert.equal((await mf.dispatchFetch(`https://${host}/smoke/v1`)).status,503);
 }
});
