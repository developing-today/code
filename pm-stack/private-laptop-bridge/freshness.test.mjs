import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fetchQueueSnapshot, unavailableQueueSnapshot } from '../aggregator/src/queue.ts';
import { publishBridgeQueueSnapshot } from '../aggregator/src/bridge-snapshot.ts';
import { createAdapters, readQueueSnapshot } from './adapters.mjs';
import { createClient } from './client.mjs';
import { createCaller } from './caller.mjs';
import worker, { PrivateLaptopBridge } from './worker.mjs';
import { operations, statuses, b64url } from './protocol.mjs';

const NOW = 1800000000000;
const rawIdle = () => ({ queue: 'repo-health-code-pilot', ...Object.fromEntries(statuses.map(s => [s, 0])) });
const path = home => join(home, '.local/state/pm-aggregator/bridge-queue.json');

// Real broker collector, producer, private file, adapter, client, Worker and
// caller are exercised. Only HTTP broker data and atomic DO storage are fake.
class Storage {
  value; tail = Promise.resolve();
  transaction(fn) {
    const promise = this.tail.then(async () => {
      let value = structuredClone(this.value);
      const result = await fn({ get: async () => structuredClone(value), put: async (_, next) => { value = structuredClone(next); }, setAlarm: async () => {} });
      this.value = value; return result;
    });
    this.tail = promise.catch(() => {}); return promise;
  }
}
async function harness(t) {
  let clock = NOW;
  t.mock.method(Date, 'now', () => clock);
  const home = await mkdtemp(join(tmpdir(), 'bridge-freshness-'));
  t.after(() => rm(home, { recursive: true, force: true }));
  const keys = {}, principals = [];
  for (const [id, role] of [['owner','caller'], ['laptop','device']]) {
    keys[id] = await crypto.subtle.generateKey('Ed25519', true, ['sign','verify']);
    principals.push({ id, role, device:'amd', publicKey:b64url(await crypto.subtle.exportKey('raw', keys[id].publicKey)), scopes:[...operations], issuedAt:NOW-1000, expiresAt:NOW+1800000, revoked:false });
  }
  const env = { ENABLED:'true', BRIDGE_REGISTRY:JSON.stringify({ audience:'freshness-test', principals }) };
  const object = new PrivateLaptopBridge({ storage:new Storage() }, env);
  env.BRIDGE = { idFromName:x=>x, get:()=>({ fetch:r=>object.fetch(r) }) };
  const fetch = request => worker.fetch(request, env);
  const execute = createAdapters({ readSnapshot:()=>readQueueSnapshot(home), clock:()=>clock });
  const config = { origin:'https://bridge.invalid', audience:'freshness-test', fetch, now:()=>clock };
  const client = createClient({ ...config, principal:'laptop', privateKey:keys.laptop.privateKey, execute, getuid:()=>1000 });
  const caller = createCaller({ ...config, principal:'owner', device:'amd', privateKey:keys.owner.privateKey });
  const collect = (health={ok:true,rows:[rawIdle()]}, list={ok:true,rows:[]}, fetchedAt=clock) => fetchQueueSnapshot('http://synthetic.invalid', 'synthetic-not-a-secret', async url=>({ ok:true,status:200,json:async()=>url.endsWith('/health')?health:list }), ()=>fetchedAt);
  const publish = source => publishBridgeQueueSnapshot(source, '1', home, clock);
  const roundtrip = async (list=false) => {
    const handle = list ? await caller.fleetQueueList({limit:20}) : await caller.fleetQueueHealth();
    assert.equal(await client.step(), true);
    return caller.read(handle);
  };
  return {home,collect,publish,roundtrip,execute,client,caller,setTime:value=>{clock=value;}};
}

test('fresh explicit zero counters are observed idle; empty list stays incomplete and total unknown', async t => {
  const h=await harness(t); const source=await h.collect();
  assert.equal(source.bridgeVerified,true);
  await h.publish(source);
  const health=await h.roundtrip();
  assert.deepEqual(health.result,{ok:true,data:{...Object.fromEntries(statuses.map(s=>[s,0])),fetchedAt:NOW}});
  const list=await h.roundtrip(true);
  assert.deepEqual(list.result,{ok:true,data:{tasks:[],complete:false,total:null,fetchedAt:NOW}});
  assert.equal(JSON.parse(await readFile(path(h.home),'utf8')).available,true);
});

test('malformed raw broker omission cannot become idle through legacy zero coercion', async t => {
  const h=await harness(t);
  const cases=[{}, {ok:true}, {ok:true,rows:[{queue:'code'}]}, {ok:true,rows:[{...rawIdle(),queued:null}]}, {ok:true,rows:[{...rawIdle(),queued:-1}]}, {ok:true,rows:[{...rawIdle(),queue:''}]}, {ok:true,rows:[rawIdle(),rawIdle()]}];
  for(const health of cases) {
    const source=await h.collect(health);
    assert.equal(source.bridgeVerified,false);
    await h.publish(source);
    assert.deepEqual((await h.roundtrip()).result,{ok:false,error:'unavailable'});
    // Advance quota window; never mask a denial as evidence of stale rejection.
    h.setTime(NOW+60000);
  }
});

test('missing or malformed list envelope and unverified/truthy source never release idle', async t => {
  const h=await harness(t);
  for(const list of [{}, {ok:true}, {rows:[]}, {ok:true,rows:null}]) {
    const source=await h.collect(undefined,list);
    assert.equal(source.bridgeVerified,false);
    await h.publish(source);
    assert.deepEqual((await h.roundtrip(true)).result,{ok:false,error:'unavailable'});
  }
  const valid=await h.collect();
  for(const mutation of [{bridgeVerified:undefined},{available:'yes'},{stale:undefined},{errors:['private-error']}]) {
    await h.publish({...valid,...mutation});
    assert.deepEqual(await h.execute('fleet.queue.health',{}),{ok:false,error:'unavailable'});
  }
});

test('idle snapshot freshness boundary is explicit: exactly five minutes allowed, one ms later unavailable', async t => {
  const h=await harness(t); await h.publish(await h.collect());
  h.setTime(NOW+300000);
  assert.equal((await h.roundtrip()).result.ok,true);
  h.setTime(NOW+300001);
  assert.deepEqual((await h.roundtrip()).result,{ok:false,error:'unavailable'});
});

test('missing file and known unavailable marker are not an observed idle queue', async t => {
  const h=await harness(t);
  assert.deepEqual((await h.roundtrip()).result,{ok:false,error:'unavailable'});
  await h.publish(await h.collect());
  assert.equal((await h.roundtrip()).result.ok,true);
  await h.publish(unavailableQueueSnapshot(NOW,'not-exported'));
  assert.deepEqual((await h.roundtrip()).result,{ok:false,error:'unavailable'});
});

test('failed publisher leaves prior observation readable only until original TTL, never refreshes its timestamp', async t => {
  const h=await harness(t); await h.publish(await h.collect());
  h.setTime(NOW+10000);
  // A producer crash before atomic rename preserves the previous observation.
  await assert.rejects(h.publish(null));
  const retained=await h.roundtrip();
  assert.equal(retained.result.ok,true);assert.equal(retained.result.data.fetchedAt,NOW);
  h.setTime(NOW+300001);
  assert.deepEqual((await h.roundtrip()).result,{ok:false,error:'unavailable'});
});

test('future source timestamps never reach caller as successful idle, including tolerated device skew', async t => {
  const h=await harness(t);
  await h.publish(await h.collect(undefined,undefined,NOW+1));
  const handle=await h.caller.fleetQueueHealth();await h.client.step();
  await assert.rejects(h.caller.read(handle),error=>error.code==='stale-result');
  await h.publish(await h.collect(undefined,undefined,NOW+5001));
  assert.deepEqual((await h.roundtrip()).result,{ok:false,error:'unavailable'});
});

test('result becoming stale after device completion is denied at read rather than returned idle', async t => {
  const h=await harness(t); await h.publish(await h.collect());
  h.setTime(NOW+299999);
  const handle=await h.caller.fleetQueueHealth();await h.client.step();
  h.setTime(NOW+300001);
  await assert.rejects(h.caller.read(handle),error=>error.code==='denied');
});
