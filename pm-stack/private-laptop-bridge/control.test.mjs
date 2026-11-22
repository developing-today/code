import {test} from 'node:test';
import assert from 'node:assert/strict';
import worker, {PrivateLaptopBridge} from './worker.mjs';
import {operations, statuses, b64url, signRequest, validateOperation, validateResult, boundedText} from './protocol.mjs';

// Oracle: serialized durable transactions; rollback on failure. Instances share
// the same storage on restart. Crypto is real WebCrypto; keys never leave memory.
class Storage {
  value; tail = Promise.resolve(); alarmAt;
  transaction(fn) {
    const result = this.tail.then(async () => {
      let value = structuredClone(this.value), alarmAt = this.alarmAt;
      const out = await fn({get: async () => structuredClone(value), put: async (_, x) => { value = structuredClone(x); }, setAlarm: async x => { alarmAt = x; }});
      this.value = value; this.alarmAt = alarmAt; return out;
    });
    this.tail = result.catch(() => {}); return result;
  }
}
async function setup() {
  const keys = {}, principals = [];
  for (const [id, role] of [['alice', 'caller'], ['bob', 'caller'], ['laptop', 'device']]) {
    keys[id] = await crypto.subtle.generateKey('Ed25519', true, ['sign', 'verify']);
    principals.push({id, role, device: 'amd', publicKey: b64url(await crypto.subtle.exportKey('raw', keys[id].publicKey)), scopes: [...operations], issuedAt: Date.now() - 1000, expiresAt: Date.now() + 1800000, revoked: false});
  }
  const registry = {audience: 'offline-test', principals};
  const env = {ENABLED: 'true', BRIDGE_REGISTRY: JSON.stringify(registry)};
  const storage = new Storage(); let object = new PrivateLaptopBridge({storage}, env);
  env.BRIDGE = {idFromName: x => x, get: () => ({fetch: req => object.fetch(req)})};
  const request = (id, path, body, options = {}) => signRequest({origin: 'https://bridge.invalid', audience: registry.audience, principal: id, privateKey: keys[id].privateKey, path, body, ...options});
  const call = async (id, path, body, options) => { const r = await worker.fetch(await request(id, path, body, options), env); return {status: r.status, body: await r.json()}; };
  const update = fn => { fn(registry); env.BRIDGE_REGISTRY = JSON.stringify(registry); };
  const start = (id = 'alice', operation = 'laptop.health', args = {}) => call(id, '/v1/call', {device: 'amd', operation, args});
  return {env, storage, request, call, update, start, restart: () => { object = new PrivateLaptopBridge({storage}, env); }, alarm: () => object.alarm()};
}
const okHealth = {ok: true, data: {queueBroker: 'up'}};

test('normal caller -> outbound poll -> result -> owner read; persists through restart', async () => {
  const h = await setup(); const task = await h.start(); assert.equal(task.status, 202);
  assert.deepEqual((await h.call('alice', '/v1/read', {id: task.body.id})).body, {status: 'pending'});
  h.restart(); const {job} = (await h.call('laptop', '/v1/poll', {})).body;
  assert.equal(job.id, task.body.id);
  assert.equal((await h.call('laptop', '/v1/result', {id: job.id, lease: job.lease, result: okHealth})).status, 200);
  assert.deepEqual((await h.call('alice', '/v1/read', {id: job.id})).body, {status: 'complete', result: okHealth});
  assert.equal((await h.call('bob', '/v1/read', {id: job.id})).status, 404);
  assert.equal((await h.call('laptop', '/v1/result', {id: job.id, lease: job.lease, result: okHealth})).status, 409);
});

test('signature binds audience, principal, exact method/path/body; unknown or expired identities deny', async () => {
  const h = await setup();
  assert.equal((await h.start('alice')).status, 202); // positive control
  for (const options of [{audience: 'other'}, {now: Date.now() - 31000}, {now: Date.now() + 31000}]) assert.equal((await h.call('alice', '/v1/poll', {}, options)).status, 401);
  const req = await h.request('alice', '/v1/poll', {});
  assert.equal((await worker.fetch(new Request(req.url, {method: 'POST', headers: req.headers, body: '{"extra":1}'}), h.env)).status, 401);
  const r2 = await h.request('alice', '/v1/poll', {});
  assert.equal((await worker.fetch(new Request('https://bridge.invalid/v1/read', {method: 'POST', headers: r2.headers, body: '{}'}), h.env)).status, 401);
  h.update(r => { r.principals[0].revoked = true; });
  assert.equal((await h.call('alice', '/v1/poll', {})).status, 401);
});

test('role, scope, operation schema and forbidden argument corpus fail closed', async () => {
  const h = await setup();
  assert.equal((await h.call('alice', '/v1/poll', {})).status, 403);
  assert.equal((await h.start('laptop')).status, 403);
  for (const operation of ['exec', '/exec', 'files.read', 'laptop.health;id', '', null]) assert.equal((await h.start('alice', operation)).status, 400);
  for (const args of [{command: 'id'}, {path: '/etc/passwd'}, {url: 'https://example.com'}, [], null]) assert.equal((await h.start('alice', 'laptop.health', args)).status, 400);
  h.update(r => { r.principals[0].scopes = ['fleet.queue.health']; });
  assert.equal((await h.start()).status, 403);
});

test('replay is durable, concurrent duplicates execute once, rejected requests consume quota', async () => {
  const h = await setup(); const nonce = 'fixed-nonce'; const req = await h.request('alice', '/v1/call', {device: 'amd', operation: 'laptop.health', args: {}}, {nonce});
  const results = await Promise.all(Array.from({length: 12}, () => worker.fetch(req.clone(), h.env)));
  assert.equal(results.filter(r => r.status === 202).length, 1); assert.equal(results.filter(r => r.status === 409).length, 11);
  h.restart(); assert.equal((await worker.fetch(req.clone(), h.env)).status, 409);
  for (let n = 13; n < 60; n++) assert.equal((await h.call('alice', '/v1/poll', {})).status, 403);
  assert.equal((await h.call('alice', '/v1/poll', {})).status, 429);
  assert.equal(Object.keys(h.storage.value.jobs).length, 1);
});

test('disconnect/reconnect fences old lease; revoked caller and device cannot finish or read', async () => {
  const h = await setup(); const task = await h.start();
  const first = (await h.call('laptop', '/v1/poll', {})).body.job;
  assert.equal((await h.call('laptop', '/v1/poll', {})).body.job, null);
  // Model passage of lease deadline without sleeping or weakening production clock.
  h.storage.value.jobs[first.id].leaseUntil = Date.now() - 1;
  h.restart(); const second = (await h.call('laptop', '/v1/poll', {})).body.job;
  assert.notEqual(second.lease, first.lease);
  assert.equal((await h.call('laptop', '/v1/result', {id: first.id, lease: first.lease, result: okHealth})).status, 409);
  assert.equal((await h.call('laptop', '/v1/result', {id: second.id, lease: second.lease, result: okHealth})).status, 200);
  h.update(r => { r.principals[2].revoked = true; });
  assert.equal((await h.call('alice', '/v1/read', {id: task.body.id})).status, 410);
  assert.equal((await h.call('laptop', '/v1/poll', {})).status, 401);
  const g = await setup(); await g.start(); g.update(r => { r.principals[0].revoked = true; });
  assert.equal((await g.call('laptop', '/v1/poll', {})).body.job, null);
});

test('expiry, queue capacity, malformed results, audit minimization, disabled/storage failures', async () => {
  const h = await setup(); const start = await h.start(); const job = (await h.call('laptop', '/v1/poll', {})).body.job;
  assert.equal((await h.call('laptop', '/v1/result', {id: job.id, lease: job.lease, result: {ok: true, data: {queueBroker: 'up', secret: 'marker'}}})).status, 400);
  assert.equal(JSON.stringify(h.storage.value).includes('marker'), false);
  h.storage.value.jobs[job.id].expiresAt = Date.now() - 1;
  assert.equal((await h.call('alice', '/v1/read', {id: start.body.id})).status, 410);
  for (let n = 1; n < 8; n++) assert.equal((await h.start()).status, 202);
  assert.equal((await h.start()).status, 429);
  assert.ok(new TextEncoder().encode(JSON.stringify(h.storage.value)).byteLength < 100000);
  h.env.ENABLED = 'false'; assert.equal((await h.start()).status, 503);
  h.env.ENABLED = 'true'; h.storage.transaction = async () => { throw new Error('private path and secrets'); };
  assert.deepEqual((await h.start()).body, {error: 'storage'});
});

test('registry fails closed on absent config, shared caller/device key and long enrollment', async () => {
  for (const change of [r => {r.principals[2].publicKey = r.principals[0].publicKey;}, r => {r.principals[0].expiresAt += 3600000;}, r => {r.principals[0].id = '__proto__';}]) {
    const h = await setup(); h.update(change); assert.equal((await h.start()).status, 503);
  }
  for (const id of ['toString', 'valueOf', 'hasOwnProperty', '__defineGetter__']) {
    const g = await setup(); g.update(r => {r.principals[0].id = id;}); assert.equal((await g.start()).status, 503);
  }
  const h = await setup(); delete h.env.BRIDGE_REGISTRY; assert.equal((await h.start()).status, 503);
});

test('schema property corpus: bounds, extra keys and all eight canonical statuses', () => {
  for (let limit = -1000; limit <= 1000; limit++) assert.equal(validateOperation('fleet.queue.list', {limit}), limit >= 1 && limit <= 20);
  for (const status of statuses) {
    assert.equal(validateOperation('fleet.queue.list', {limit: 1, status}), true);
    assert.equal(validateResult('fleet.queue.list', {ok: true, data: {tasks: [{id: 1, status}], complete: false, total: null, fetchedAt: Date.now()}}), true);
  }
  for (const key of ['__proto__', 'command', 'path', 'url', 'token', 'cwd', 'shell', 'argv']) {
    const value = JSON.parse(`{"limit":1,"${key}":"marker"}`);
    assert.equal(validateOperation('fleet.queue.list', value), false);
  }
  for (const n of [-1, NaN, Infinity, 1.5, Number.MAX_SAFE_INTEGER + 1]) assert.equal(validateResult('laptop.diagnostics', {ok: true, data: {uptimeSeconds: n, load1: 1, memoryFreeBytes: 0}}), false);
});

test('alarm removes expired state without later inbound traffic', async () => {
  const h = await setup(); await h.start();
  for (const job of Object.values(h.storage.value.jobs)) job.retainUntil = 0;
  for (const k of Object.keys(h.storage.value.nonces)) h.storage.value.nonces[k] = 0;
  for (const q of Object.values(h.storage.value.quotas)) q.end = 0;
  for (const e of h.storage.value.audit) e.at = 0;
  await h.alarm(); assert.deepEqual(h.storage.value, {nonces: {}, quotas: {}, jobs: {}, audit: []});
});


test('body size, streaming deadline and header limits reject before dispatch', async () => {
  await assert.rejects(boundedText(new ReadableStream({start(c) {c.enqueue(new Uint8Array(8193));}})), /size/);
  await assert.rejects(boundedText(new ReadableStream({}), 8192, 5), /timeout/);
  const h = await setup();
  const large = await h.request('alice', '/v1/poll', {data: 'x'.repeat(8192)});
  assert.equal((await worker.fetch(large, h.env)).status, 413);
  const req = await h.request('alice', '/v1/poll', {});
  req.headers.set('content-length', '90000000000');
  assert.equal((await worker.fetch(req, h.env)).status, 413);
  const bad = await h.request('alice', '/v1/poll', {});
  bad.headers.set('x-bridge-signature', 'x'.repeat(9000));
  assert.equal((await worker.fetch(bad, h.env)).status, 401);
  assert.equal(h.storage.value, undefined);
});

test('scope removal invalidates outstanding completion; role changes invalidate result reads', async () => {
  const h = await setup(); await h.start();
  const job = (await h.call('laptop', '/v1/poll', {})).body.job;
  h.update(r => {r.principals[0].scopes = ['fleet.queue.health'];});
  assert.equal((await h.call('laptop', '/v1/result', {id: job.id, lease: job.lease, result: okHealth})).status, 409);
  const g = await setup(); await g.start(); const j = (await g.call('laptop', '/v1/poll', {})).body.job;
  await g.call('laptop', '/v1/result', {id: j.id, lease: j.lease, result: okHealth});
  g.update(r => {r.principals[2].role = 'caller';});
  assert.equal((await g.call('alice', '/v1/read', {id: j.id})).status, 410);
});

test('source timestamps fail closed at completion and read; alarms are not postponed by new requests', async () => {
  const h = await setup(); await h.start('alice', 'fleet.queue.health');
  const firstAlarm = h.storage.alarmAt;
  const job = (await h.call('laptop', '/v1/poll', {})).body.job;
  assert.equal(h.storage.alarmAt, firstAlarm);
  const data = Object.fromEntries(statuses.map(s => [s, 0]));
  assert.equal((await h.call('laptop', '/v1/result', {id: job.id, lease: job.lease, result: {ok: true, data: {...data, fetchedAt: Date.now() - 300001}}})).status, 400);
  assert.equal((await h.call('laptop', '/v1/result', {id: job.id, lease: job.lease, result: {ok: true, data: {...data, fetchedAt: Date.now() + 60000}}})).status, 400);
  assert.equal((await h.call('laptop', '/v1/result', {id: job.id, lease: job.lease, result: {ok: true, data: {...data, fetchedAt: Date.now()}}})).status, 200);
  h.storage.value.jobs[job.id].result.data.fetchedAt = Date.now() - 300001;
  assert.equal((await h.call('alice', '/v1/read', {id: job.id})).status, 410);
  h.storage.value.audit.unshift({at: Date.now() - 180001, principal: 'alice', role: 'caller', route: '/v1/read', status: 200});
  await h.call('alice', '/v1/read', {id: job.id});
  assert.ok(h.storage.value.audit.every(e => e.at > Date.now() - 180000));
});
