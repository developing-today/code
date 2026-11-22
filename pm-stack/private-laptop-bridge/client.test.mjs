import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, symlink, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createClient, parseArgs, requireNonRoot, validateConfig, BridgeDenied } from './client.mjs';
import { createAdapters, projectQueueSnapshot, readQueueSnapshot } from './adapters.mjs';

const statuses = ['queued','claimed','parked','handed_off','blocked','done','cancelled','failed'];
const now = 1800000000000;
const snapshot = () => ({ version:1, fetchedAt:now, available:true, stale:false, complete:false, total:null, tasks: [{id:1,status:'queued',priority:0},{id:2,status:'cancelled',priority:null}], counts:Object.fromEntries(statuses.map(s => [s, s === 'queued' ? 1 : 0])) });
const json = body => new Response(JSON.stringify(body), { headers:{'content-type':'application/json'} });
const keys = await crypto.subtle.generateKey('Ed25519', true, ['sign','verify']);
const base = { origin:'https://bridge.invalid', audience:'test-bridge', principal:'amd-device', privateKey:keys.privateKey, now:()=>now, getuid:()=>1000 };
const job = (overrides = {}) => ({ id:'job-1', lease:'lease-1', operation:'laptop.health', args:{}, expiresAt:now+60000, leaseExpiresAt:now+15000, ...overrides });

test('oracle: CLI grammar rejects every undeclared argument before activity', () => {
  assert.deepEqual(parseArgs(['--help']), {help:true});
  assert.deepEqual(parseArgs(['--config','/owned/device.json']), {config:'/owned/device.json'});
  for (const argv of [[],['--config'],['--config=x'],['x'],['--help','x'],['--config','relative'],['--config','/x','--shell']]) assert.throws(()=>parseArgs(argv), /usage/);
  assert.throws(()=>requireNonRoot(()=>0), /non-root/);
  assert.throws(()=>requireNonRoot(null), /non-root/);
});

test('oracle: non-root gate runs before network and invalid origins cannot be configured', () => {
  let calls=0;
  assert.throws(()=>createClient({...base,getuid:()=>0,fetch:()=>calls++}), /non-root/);
  assert.equal(calls,0);
  for (const origin of ['http://bridge.invalid','https://bridge.invalid/path','https://user:pass@bridge.invalid','https://bridge.invalid/']) assert.throws(()=>createClient({...base,origin}));
});

test('oracle: configuration contains exactly owner provisioned Ed25519 identity', async () => {
  const exported=await crypto.subtle.exportKey('jwk', keys.privateKey);
  const privateKey=Object.fromEntries(['kty','crv','x','d'].map(k=>[k,exported[k]]));
  const config={origin:base.origin,audience:base.audience,principal:base.principal,privateKey};
  assert.equal(validateConfig(config),config);
  assert.throws(()=>validateConfig({...config,shell:'/bin/sh'}));
  assert.throws(()=>validateConfig({...config,privateKey:{...privateKey,kty:'RSA'}}));
});

test('oracle: exact signed outbound poll, allowlisted adapter, and completion', async () => {
  const requests=[];
  const client=createClient({...base, execute:async(op,args)=>{assert.equal(op,'laptop.health');assert.deepEqual(args,{});return {ok:true,data:{queueBroker:'up'}};}, fetch:async(request,options)=>{
    assert.equal(options.redirect,'error');assert.ok(options.signal);assert.ok(request.headers.get('x-bridge-signature'));
    requests.push({path:new URL(request.url).pathname,body:await request.json()});
    return requests.length===1?json({job:job()}):json({ok:true});
  }});
  assert.equal(await client.step(),true);
  assert.deepEqual(requests,[{path:'/v1/poll',body:{}},{path:'/v1/result',body:{id:'job-1',lease:'lease-1',result:{ok:true,data:{queueBroker:'up'}}}}]);
});

test('oracle: unknown capabilities/arguments/expiry never reach adapters', async () => {
  let executes=0;
  const cases=[job({operation:'shell'}),job({args:{url:'http://metadata.invalid'}}),job({expiresAt:now}),job({expiresAt:now+60001}),job({leaseExpiresAt:now}),job({leaseExpiresAt:now+15001}),{...job(),command:'id'},job({lease:'../escape'})];
  for (const item of cases) {
    const client=createClient({...base,execute:async()=>{executes++;},fetch:async()=>json({job:item})});
    await assert.rejects(client.step(),/invalid-job/);
  }
  assert.equal(executes,0);
});

test('oracle: explicit authorization denial terminates instead of reconnecting', async () => {
  for (const status of [401,403,410]) {
    let slept=0;
    const client=createClient({...base,fetch:async()=>new Response('',{status}),sleep:async()=>slept++});
    await assert.rejects(client.run(),BridgeDenied);assert.equal(slept,0);
  }
});

test('oracle: disconnect backoff is bounded and successful reconnect resets it', async () => {
  const abort=new AbortController();const waits=[];let calls=0;
  const client=createClient({...base,fetch:async()=>{calls++;if(calls<=7)throw new Error('offline');return json({job:null});},sleep:async(ms)=>{waits.push(ms);if(calls===8)abort.abort();}});
  await client.run(abort.signal);
  assert.deepEqual(waits,[1000,2000,4000,8000,16000,30000,30000,1000]);
});

test('oracle: redirects and overlarge responses fail closed', async () => {
  for (const response of [new Response('',{status:302,headers:{location:'https://evil.invalid'}}),new Response('x'.repeat(16385))]) {
    const client=createClient({...base,fetch:async()=>response});
    await assert.rejects(client.step());
  }
});

test('oracle: response stream deadline cancels stalled reads', async () => {
  let canceled=false;
  const keepalive=setTimeout(()=>{},1000);
  try {
    const client=createClient({...base,requestTimeoutMs:15,fetch:async()=>new Response(new ReadableStream({cancel(){canceled=true;}}))});
    await assert.rejects(client.step()); assert.equal(canceled,true);
  } finally {clearTimeout(keepalive);}
});

test('oracle: expiry during execution suppresses result and secret-shaped output is rejected', async () => {
  let clock=now,requests=0;
  const client=createClient({...base,now:()=>clock,fetch:async()=>{requests++;return json({job:job()});},execute:async()=>{clock+=60001;return {ok:true,data:{queueBroker:'up'}};}});
  await assert.rejects(client.step(),/expired-job/);assert.equal(requests,1);
  const leaked=createClient({...base,fetch:async()=>json({job:job()}),execute:async()=>({ok:true,data:{queueBroker:'up',token:'never-return'}})});
  await assert.rejects(leaked.step(),/invalid-result/);
});

test('oracle: real snapshot schema projects only IDs/status with explicit partial semantics', () => {
  const result=projectQueueSnapshot(snapshot(),'fleet.queue.list',{limit:1},now);
  assert.deepEqual(result,{ok:true,data:{tasks:[{id:1,status:'queued'}],complete:false,total:null}});
  assert.deepEqual(projectQueueSnapshot(snapshot(),'fleet.queue.health',{},now),{ok:true,data:snapshot().counts});
  assert.deepEqual(projectQueueSnapshot(snapshot(),'fleet.queue.list',{limit:20,status:'cancelled'},now).data.tasks,[{id:2,status:'cancelled'}]);
});

test('invariant/property corpus: malformed/stale snapshots never release source text', () => {
  const cases=[null,{}, {...snapshot(),fetchedAt:now-300001},{...snapshot(),fetchedAt:now+5001},{...snapshot(),available:false},{...snapshot(),tasks:[{id:1,status:'queued',priority:null,payload:'secret'}]},{...snapshot(),counts:{...snapshot().counts,queued:-1}}];
  for(let i=0;i<200;i++)cases.push({...snapshot(),tasks:[{id:i%2?-i:1.5,status:'queued',priority:null}]});
  for(const source of cases) {
    const result=projectQueueSnapshot(source,'fleet.queue.list',{limit:20},now);
    assert.equal(result.ok,false);assert.ok(!JSON.stringify(result).includes('secret'));
  }
});

test('oracle: production reader uses fixed file, rejects symlink and size overflow', async () => {
  const home=await mkdtemp(join(tmpdir(),'bridge-fixture-'));const directory=join(home,'.local/state/pm-aggregator');const path=join(directory,'bridge-queue.json');
  try {
    await mkdir(directory,{recursive:true});await writeFile(path,JSON.stringify(snapshot()));
    assert.deepEqual(await readQueueSnapshot(home),snapshot());
    await rm(path);await symlink('/etc/passwd',path);await assert.rejects(readQueueSnapshot(home));
    await rm(path);await writeFile(path,'x'.repeat(1024*1024+1));await assert.rejects(readQueueSnapshot(home));
  } finally {await rm(home,{recursive:true,force:true});}
});

test('oracle: adapters return only bounded categorical/scalar data and swallow raw errors', async () => {
  const execute=createAdapters({readSnapshot:async()=>snapshot(),clock:()=>now,probe:async()=>true,diagnostics:()=>({uptimeSeconds:10,load1:0.5,memoryAvailableBytes:1000})});
  assert.deepEqual(await execute('laptop.health',{}),{ok:true,data:{queueBroker:'up'}});
  assert.equal((await execute('fleet.queue.list',{limit:20})).data.tasks.length,2);
  assert.equal((await execute('laptop.diagnostics',{})).data.uptimeSeconds,10);
  const broken=createAdapters({readSnapshot:async()=>{throw new Error('sensitive-source-text');}});
  assert.deepEqual(await broken('fleet.queue.list',{limit:1}),{ok:false,error:'unavailable'});
  assert.deepEqual(await execute('shell',{command:'id'}),{ok:false,error:'invalid-source'});
});
