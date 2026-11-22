import {test} from 'node:test';
import assert from 'node:assert/strict';
import worker, {PrivateLaptopBridge} from './worker.mjs';
import {b64url, signRequest} from './protocol.mjs';

// Oracle: an independent, deterministic transition model. The generated traces
// never edit implementation state or deadlines. Only Date advances; a reconnect
// reconstructs the object with its unchanged durable storage. Keys stay in RAM.
class Storage {
  value;
  async transaction(fn) {
    let value = structuredClone(this.value);
    const result = await fn({get: async () => structuredClone(value), put: async (_, next) => { value = structuredClone(next); }, setAlarm: async () => {}});
    this.value = value;
    return result;
  }
}
function random(seed) {
  return () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed; };
}
const good = {ok:true, data:{queueBroker:'up'}};

test('model: 50 seeded call/poll/lease/replay/revoke/reconnect traces preserve authorization', async t => {
  const start = 1800000000000;
  t.mock.timers.enable({apis:['Date'], now:start});
  const keys = {}, definitions = [];
  for (const [id, role] of [['alice','caller'], ['bob','caller'], ['laptop','device']]) {
    keys[id] = await crypto.subtle.generateKey('Ed25519', true, ['sign','verify']);
    definitions.push({id, role, device:'amd', publicKey:b64url(await crypto.subtle.exportKey('raw',keys[id].publicKey)), scopes:['laptop.health'], issuedAt:start-1000, expiresAt:start+3599000, revoked:false});
  }
  const coverage={accepted:0,leased:0,completed:0,staleLeaseDenied:0,replayDenied:0,completeRead:0,ownerDenied:0};
  for (let seed=1; seed<=50; seed++) {
    let now = start;
    t.mock.timers.setTime(now);
    const rng=random(seed), store=new Storage();
    const config={audience:'model-test',principals:structuredClone(definitions)};
    const env={ENABLED:'true',BRIDGE_REGISTRY:JSON.stringify(config)};
    let object=new PrivateLaptopBridge({storage:store},env);
    env.BRIDGE={idFromName:x=>x,get:()=>({fetch:req=>object.fetch(req)})};
    const revoked={alice:false,bob:false,laptop:false};
    const jobs=new Map(), leases=[], replays=[];
    let nextNonce=0;
    const make=(principal,path,body)=>signRequest({origin:'https://bridge.invalid',audience:'model-test',principal,privateKey:keys[principal].privateKey,path,body,now,nonce:`seed-${seed}-${nextNonce++}`});
    const send=async request=>{const response=await worker.fetch(request.clone(),env);return {status:response.status,body:await response.json()};};
    const gc=()=>{for(const [id,j] of jobs)if(j.retainUntil<=now)jobs.delete(id);};
    const live=j=>j && j.expiresAt>now && !revoked.alice;
    const check=(got,want,step,action)=>assert.equal(got,want,`seed=${seed} step=${step} action=${action}`);
    for(let step=0;step<40;step++) {
      const action=rng()%9;
      if(action===0) { // The model owns deadline arithmetic, not worker state.
        now += [1000,16000,65000][rng()%3]; t.mock.timers.setTime(now); continue;
      }
      if(action===1) {
        const principal=rng()%2?'alice':'laptop'; revoked[principal]=!revoked[principal];
        config.principals.find(p=>p.id===principal).revoked=revoked[principal];
        env.BRIDGE_REGISTRY=JSON.stringify(config); continue;
      }
      if(action===2) { object=new PrivateLaptopBridge({storage:store},env); continue; }
      if(action===3 || jobs.size===0) {
        const request=await make('alice','/v1/call',{device:'amd',operation:'laptop.health',args:{}});
        if(!revoked.alice)gc();
        const expected=revoked.alice?401:revoked.laptop?403:jobs.size>=8?429:202;
        const got=await send(request);check(got.status,expected,step,'call');
        if(got.status===202) {
          coverage.accepted++;
          jobs.set(got.body.id,{expiresAt:now+60000,retainUntil:now+120000,lease:null,leaseUntil:0,assignee:null,done:false});
          replays.push({request,at:now});
        }
        continue;
      }
      if(action===4) {
        if(!revoked.laptop)gc();
        const eligible=[...jobs.entries()].find(([,j])=>live(j)&&!j.done&&j.leaseUntil<=now);
        const got=await send(await make('laptop','/v1/poll',{}));
        check(got.status,revoked.laptop?401:200,step,'poll');
        if(got.status===200) {
          if(!eligible)assert.equal(got.body.job,null);
          else {
            coverage.leased++;
            const [id,j]=eligible;assert.equal(got.body.job.id,id);
            assert.equal(got.body.job.leaseExpiresAt,Math.min(now+15000,j.expiresAt));
            j.lease=got.body.job.lease;j.leaseUntil=Math.min(now+15000,j.expiresAt);j.assignee='laptop';
            leases.push({id,lease:j.lease});
          }
        }
        continue;
      }
      if(action===5 && leases.length) {
        const old=leases[rng()%leases.length];
        if(!revoked.laptop)gc();
        const j=jobs.get(old.id);
        const valid=live(j)&&!j.done&&j.lease===old.lease&&j.leaseUntil>now;
        const got=await send(await make('laptop','/v1/result',{...old,result:good}));
        check(got.status,revoked.laptop?401:valid?200:409,step,'finish');
        if(got.status===200){j.done=true;coverage.completed++;}
        if(got.status===409)coverage.staleLeaseDenied++;
        continue;
      }
      if(action===6 && replays.length) {
        const replay=replays[rng()%replays.length];
        const authenticated=!revoked.alice&&now-replay.at<=30000;
        if(authenticated)gc();
        const before=jobs.size;
        const got=await send(replay.request);
        check(got.status,authenticated?409:401,step,'replay');
        if(got.status===409)coverage.replayDenied++;
        assert.equal(Object.keys(store.value?.jobs??{}).length,before);
        continue;
      }
      const principal=rng()%2?'alice':'bob';
      if(!revoked[principal])gc();
      const entries=[...jobs.entries()];
      const [id,j]=entries.length?entries[rng()%entries.length]:['missing',null];
      const got=await send(await make(principal,'/v1/read',{id}));
      const expected=revoked[principal]?401:!j||principal!=='alice'?404:!live(j)||(j.assignee&&revoked.laptop)?410:200;
      check(got.status,expected,step,'read');
      if(got.status===404&&principal==='bob')coverage.ownerDenied++;
      if(got.status===200&&j.done)coverage.completeRead++;
      if(got.status===200)assert.deepEqual(got.body,j.done?{status:'complete',result:good}:{status:'pending'});
    }
  }
  // Guard against vacuous seeded traces that happen never to reach a state.
  for(const [state,count] of Object.entries(coverage))assert.ok(count>0,`missing generated coverage: ${state}`);
  t.diagnostic(JSON.stringify(coverage));
});
