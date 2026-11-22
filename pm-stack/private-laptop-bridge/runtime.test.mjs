// Real workerd + SQLite Durable Object acceptance tests. No deployment, external
// requests or persistent private keys. Oracle: protocol outcomes and persistent
// replay/lease invariants, exercised through the exported Worker entry point.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { Miniflare } from 'miniflare';

const here = dirname(fileURLToPath(import.meta.url));
const source = here;
const clientSource = here;
const { signRequest, b64url, operations } = await import(pathToFileURL(join(source, 'protocol.mjs')));
const origin = 'https://private-bridge.invalid';
const audience = 'offline-acceptance';
const result = { ok: true, data: { queueBroker: 'up' } };

async function harness(t) {
  const dir = await mkdtemp(join(tmpdir(), 'private-bridge-runtime-'));
  const keys = {};
  const principals = [];
  const now = Date.now();
  for (const [id, role, device] of [['caller','caller','amd'], ['other','caller','amd'], ['device','device','amd'], ['wrong-device','device','other']]) {
    keys[id] = await crypto.subtle.generateKey('Ed25519', true, ['sign','verify']);
    principals.push({ id, role, device, publicKey: b64url(await crypto.subtle.exportKey('raw', keys[id].publicKey)), scopes: [...operations], issuedAt: now - 1000, expiresAt: now + 120000, revoked: false });
  }
  const registry = { audience, principals };
  let mf;
  const options = () => ({
    name: 'private-laptop-bridge-offline',
    host: '127.0.0.1',
    modules: true,
    scriptPath: join(source, 'worker.mjs'),
    modulesRules: [{ type: 'ESModule', include: ['**/*.mjs'] }],
    compatibilityDate: '2026-07-30',
    bindings: { ENABLED: 'true', BRIDGE_REGISTRY: JSON.stringify(registry) },
    durableObjects: { BRIDGE: { className: 'PrivateLaptopBridge', useSQLite: true } },
    durableObjectsPersist: dir,
    // All calls enter through dispatchFetch; the actual application has no
    // upstream fetch. Block external I/O if future code accidentally adds one.
    outboundService: () => new Response('external I/O forbidden in offline test', { status: 599 }),
  });
  async function start() { mf = new Miniflare(options()); await mf.ready; }
  await start();
  t.after(async () => { await mf?.dispose(); await rm(dir, { recursive: true, force: true }); });
  async function dispatch(request, init = {}) {
    return mf.dispatchFetch(request.url, { method: request.method, headers: Object.fromEntries(request.headers), body: await request.text(), ...init });
  }
  async function signed(id, path, body, overrides = {}) {
    return signRequest({ origin, audience, principal: id, privateKey: keys[id].privateKey, path, body, ...overrides });
  }
  async function send(id, path, body, overrides = {}) {
    const res = await dispatch(await signed(id, path, body, overrides));
    return { status: res.status, body: await res.json() };
  }
  async function submit(id = 'caller', operation = 'laptop.health', args = {}) {
    const reply = await send(id, '/v1/call', { device: 'amd', operation, args });
    assert.equal(reply.status, 202, JSON.stringify(reply));
    return reply.body;
  }
  return {
    keys, registry, signed, dispatch, send, submit,
    async restart() { await mf.dispose(); await start(); },
    async reconfigure() { await mf.setOptions(options()); await mf.ready; },
  };
}

test('workerd: caller -> device -> result read, with ownership and strict output rejection', async t => {
  const h = await harness(t);
  const call = await h.submit();
  assert.deepEqual(await h.send('caller', '/v1/read', { id: call.id }), { status: 200, body: { status: 'pending' } });
  assert.equal((await h.send('other', '/v1/read', { id: call.id })).status, 404);
  const { body: { job }, status } = await h.send('device', '/v1/poll', {});
  assert.equal(status, 200);
  assert.equal(job.id, call.id);
  assert.equal(job.operation, 'laptop.health');
  assert.deepEqual(await h.send('wrong-device', '/v1/poll', {}), { status: 200, body: { job: null } });
  assert.equal((await h.send('device', '/v1/result', { id: job.id, lease: job.lease, result: { ok: true, data: { queueBroker: 'up', token: 'must-not-leave-device' } } })).status, 400);
  assert.equal((await h.send('device', '/v1/result', { id: job.id, lease: job.lease, result })).status, 200);
  assert.deepEqual(await h.send('caller', '/v1/read', { id: call.id }), { status: 200, body: { status: 'complete', result } });
  assert.equal((await h.send('device', '/v1/result', { id: job.id, lease: job.lease, result })).status, 409);
});

test('workerd: concurrent replay has one winner and nonce survives runtime restart', async t => {
  const h = await harness(t);
  const request = await h.signed('caller', '/v1/call', { device: 'amd', operation: 'laptop.health', args: {} });
  const replay = request.clone();
  const restartReplay = request.clone();
  const replies = await Promise.all([h.dispatch(request), h.dispatch(replay)]);
  assert.deepEqual(replies.map(r => r.status).sort(), [202, 409]);
  await h.restart();
  assert.equal((await h.dispatch(restartReplay)).status, 409);
  const { body: { job } } = await h.send('device', '/v1/poll', {});
  assert.ok(job, 'the accepted RPC also survives runtime restart');
  assert.deepEqual((await h.send('device', '/v1/poll', {})).body, { job: null });
});

test('workerd: signature binds role, audience, bytes and freshness; unsupported operations denied', async t => {
  const h = await harness(t);
  assert.equal((await h.send('device', '/v1/call', { device: 'amd', operation: 'laptop.health', args: {} })).status, 403);
  assert.equal((await h.send('caller', '/v1/poll', {})).status, 403);
  assert.equal((await h.send('caller', '/v1/read', { id: 'missing' }, { audience: 'another-audience' })).status, 401);
  assert.equal((await h.send('caller', '/v1/read', { id: 'missing' }, { now: Date.now() - 31000 })).status, 401);
  const valid = await h.signed('caller', '/v1/call', { device: 'amd', operation: 'laptop.health', args: {} });
  assert.equal((await h.dispatch(valid, { body: '{"device":"amd","operation":"shell","args":{}}' })).status, 401);
  for (const [operation, args] of [['shell', {}], ['laptop.health', { url: 'http://127.0.0.1/secrets' }], ['fleet.queue.list', { limit: 21 }], ['fleet.queue.list', { limit: 1, query: 'arbitrary SQL' }]]) {
    assert.equal((await h.send('caller', '/v1/call', { device: 'amd', operation, args })).status, 400);
  }
  assert.equal((await h.send('caller', '/v1/call', { device: 'other', operation: 'laptop.health', args: {} })).status, 403);
});

test('workerd: caller and device revocation deny delivery and completed result disclosure', async t => {
  const h = await harness(t);
  const pending = await h.submit();
  h.registry.principals.find(p => p.id === 'caller').revoked = true;
  await h.reconfigure();
  assert.equal((await h.send('caller', '/v1/read', { id: pending.id })).status, 401);
  assert.deepEqual((await h.send('device', '/v1/poll', {})).body, { job: null });
  h.registry.principals.find(p => p.id === 'caller').revoked = false;
  await h.reconfigure();
  const { body: { job } } = await h.send('device', '/v1/poll', {});
  assert.equal(job.id, pending.id);
  assert.equal((await h.send('device', '/v1/result', { id: job.id, lease: job.lease, result })).status, 200);
  h.registry.principals.find(p => p.id === 'device').revoked = true;
  await h.reconfigure();
  assert.equal((await h.send('device', '/v1/poll', {})).status, 401);
  assert.equal((await h.send('device', '/v1/result', { id: job.id, lease: job.lease, result })).status, 401);
  assert.equal((await h.send('caller', '/v1/read', { id: job.id })).status, 410);
  assert.equal((await h.send('caller', '/v1/call', { device: 'amd', operation: 'laptop.health', args: {} })).status, 403);
});

test('workerd: registry expiration and scope removal fail closed on stored requests', async t => {
  const h = await harness(t);
  const pending = await h.submit();
  h.registry.principals.find(p => p.id === 'caller').scopes = ['laptop.diagnostics'];
  await h.reconfigure();
  assert.equal((await h.send('caller', '/v1/read', { id: pending.id })).status, 410);
  assert.deepEqual((await h.send('device', '/v1/poll', {})).body, { job: null });
  const p = h.registry.principals.find(p => p.id === 'caller');
  p.issuedAt = Date.now() - 10000;
  p.expiresAt = Date.now() - 1000;
  await h.reconfigure();
  assert.equal((await h.send('caller', '/v1/read', { id: pending.id })).status, 401);
});

test('workerd: signed quotas, bounded bodies and finite RPC capacity', async t => {
  const h = await harness(t);
  const oversized = await h.send('caller', '/v1/read', { id: 'x'.repeat(9000) });
  assert.equal(oversized.status, 413);
  for (let i = 0; i < 60; i++) assert.equal((await h.send('other', '/v1/read', { id: 'missing' })).status, 404);
  assert.equal((await h.send('other', '/v1/read', { id: 'missing' })).status, 429);
  for (let i = 0; i < 8; i++) await h.submit();
  assert.equal((await h.send('caller', '/v1/call', { device: 'amd', operation: 'laptop.health', args: {} })).status, 429);
});

test('workerd: lost poll reconnect redelivers read after lease expiry and fences old completion', async t => {
  const h = await harness(t);
  const pending = await h.submit();
  const first = (await h.send('device', '/v1/poll', {})).body.job;
  assert.equal(first.id, pending.id);
  await h.restart();
  assert.deepEqual((await h.send('device', '/v1/poll', {})).body, { job: null });
  // This intentionally waits on the actual workerd clock, not a mocked timer.
  await delay(15250);
  const second = (await h.send('device', '/v1/poll', {})).body.job;
  assert.equal(second.id, first.id);
  assert.notEqual(second.lease, first.lease);
  assert.equal((await h.send('device', '/v1/result', { id: first.id, lease: first.lease, result })).status, 409);
  assert.equal((await h.send('device', '/v1/result', { id: second.id, lease: second.lease, result })).status, 200);
  assert.deepEqual((await h.send('caller', '/v1/read', { id: pending.id })).body, { status: 'complete', result });
});

test('workerd: actual non-root laptop client completes each named read through Worker', async t => {
  const { createClient } = await import(pathToFileURL(join(clientSource, 'client.mjs')));
  const { createCaller } = await import('./caller.mjs');
  const h = await harness(t);
  const { createAdapters, readQueueSnapshot, queueBrokerAlive } = await import('./adapters.mjs');
  const { publishBridgeQueueSnapshot } = await import('../aggregator/src/bridge-snapshot.ts');
  const home = await mkdtemp(join(tmpdir(), 'private-bridge-source-'));
  const fetchedAt = Date.now();
  const { fetchQueueSnapshot } = await import('../aggregator/src/queue.ts');
  const counts = { queue: 'synthetic-code-pilot', queued: 1, claimed: 0, parked: 0, handed_off: 0, blocked: 0, done: 0, cancelled: 0, failed: 0 };
  const snapshot = await fetchQueueSnapshot('http://synthetic-broker.invalid', 'synthetic-not-a-secret', async url => ({
    ok: true, status: 200, json: async () => url.endsWith('/health') ? {ok: true, rows: [counts]} : {
      ok: true, rows: [{ id: 42, queue: 'synthetic-code-pilot', status: 'queued', priority: 1, title: 'must-not-export', payload: { token: 'must-not-export' } }],
    },
  }), () => fetchedAt);
  await publishBridgeQueueSnapshot(snapshot, '1', home, fetchedAt);
  assert.equal((await readFile(join(home, '.local/state/pm-aggregator/bridge-queue.json'), 'utf8')).includes('must-not-export'), false);
  await mkdir(join(home, '.config/queue-broker'), { recursive: true, mode: 0o700 });
  let healthRequests = 0;
  const broker = createServer((request, response) => {
    healthRequests++;
    assert.equal(request.method, 'GET'); assert.equal(request.url, '/healthz');
    assert.equal(request.headers['x-agent-token'], undefined);
    response.end('{"ok":true}');
  });
  await new Promise(resolve => broker.listen(join(home, '.config/queue-broker/broker.sock'), resolve));
  t.after(async () => { await new Promise(resolve => broker.close(resolve)); await rm(home, { recursive: true, force: true }); });
  const samples = {
    'fleet.queue.list': { ok: true, data: { tasks: [{ id: 42, status: 'queued' }], complete: false, total: null, fetchedAt } },
    'fleet.queue.health': { ok: true, data: { queued: 1, claimed: 0, parked: 0, handed_off: 0, blocked: 0, done: 0, cancelled: 0, failed: 0, fetchedAt } },
    'laptop.health': result,
    'laptop.diagnostics': { ok: true, data: { uptimeSeconds: 123, load1: 0.25, memoryFreeBytes: 1024 } },
  };
  const seen = [];
  const adapters = createAdapters({ readSnapshot: () => readQueueSnapshot(home), probe: () => queueBrokerAlive(home), diagnostics: () => samples['laptop.diagnostics'].data });
  const client = createClient({ origin, audience, principal: 'device', privateKey: h.keys.device.privateKey, getuid: () => 1000,
    fetch: async (request, init) => { assert.equal(init.redirect, 'error'); return h.dispatch(request); },
    execute: async (operation, args) => { seen.push({ operation, args }); return adapters(operation, args); },
  });
  const caller = createCaller({ origin, audience, principal: 'caller', device: 'amd', privateKey: h.keys.caller.privateKey, fetch: request => h.dispatch(request) });
  const methods = { 'fleet.queue.list': 'fleetQueueList', 'fleet.queue.health': 'fleetQueueHealth', 'laptop.health': 'laptopHealth', 'laptop.diagnostics': 'laptopDiagnostics' };
  for (const operation of operations) {
    const args = operation === 'fleet.queue.list' ? { limit: 1, status: 'queued' } : {};
    const call = operation === 'fleet.queue.list' ? await caller[methods[operation]](args) : await caller[methods[operation]]();
    assert.equal(await client.step(), true);
    assert.deepEqual(await caller.wait(call), { status: 'complete', handle: call, result: samples[operation] });
  }
  assert.deepEqual(seen.map(x => x.operation), operations);
  assert.equal(await client.step(), false);
  assert.equal(healthRequests, 1);
});

test('caller SDK: validates before I/O and never retries uncertain submissions', async () => {
  const { createCaller, CallerError } = await import('./caller.mjs');
  const keys = await crypto.subtle.generateKey('Ed25519', true, ['sign','verify']);
  const base = { origin, audience, principal: 'caller', device: 'amd', privateKey: keys.privateKey };
  assert.throws(() => createCaller({ ...base, origin: 'http://localhost' }), /invalid-config/);
  let calls = 0;
  const caller = createCaller({ ...base, fetch: async () => { calls++; throw new Error('secret-bearing transport detail'); } });
  await assert.rejects(caller.fleetQueueList({ limit: 1, command: 'anything' }), { code: 'invalid-operation' });
  assert.equal(calls, 0);
  await assert.rejects(caller.laptopHealth(), error => error instanceof CallerError && error.code === 'submit-outcome-unknown' && !error.message.includes('secret'));
  assert.equal(calls, 1);
  await assert.rejects(caller.laptopHealth({ signal: AbortSignal.abort() }), { code: 'cancelled' });
  assert.equal(calls, 1);
});

test('caller SDK: uncertain reads preserve accepted handle and malformed results are refused', async () => {
  const { createCaller } = await import('./caller.mjs');
  const keys = await crypto.subtle.generateKey('Ed25519', true, ['sign','verify']);
  const handle = { id: 'test-id', operation: 'laptop.health', expiresAt: Date.now() + 60000, args: {} };
  let mode = 'failed';
  const caller = createCaller({ origin, audience, principal: 'caller', device: 'amd', privateKey: keys.privateKey,
    fetch: async () => {
      if (mode === 'failed') throw new Error('private stack trace');
      if (mode === 'extra') return Response.json({ status: 'complete', result: { ok: true, data: { queueBroker: 'up', raw: 'forbidden' } } });
      if (mode === 'oversize') return new Response('x'.repeat(9000));
      if (mode === 'redirect') return new Response('', { status: 302, headers: { location: 'https://elsewhere.invalid' } });
      return Response.json({ status: 'complete', result });
    },
  });
  await assert.rejects(caller.read(handle), error => error.code === 'unavailable' && error.handle === handle);
  for (mode of ['extra','oversize','redirect']) await assert.rejects(caller.read(handle), error => error.code === 'invalid-response' && error.handle === handle);
  mode = 'good';
  assert.deepEqual(await caller.read(handle), { status: 'complete', handle, result });
});

test('caller SDK: transport and streamed response deadlines are bounded', async () => {
  const { createCaller } = await import('./caller.mjs');
  const keys = await crypto.subtle.generateKey('Ed25519', true, ['sign','verify']);
  const base = { origin, audience, principal: 'caller', device: 'amd', privateKey: keys.privateKey, requestTimeoutMs: 30 };
  const silent = createCaller({ ...base, fetch: async () => new Promise(() => {}) });
  await assert.rejects(silent.laptopHealth(), { code: 'submit-outcome-unknown' });
  let cancelled = false;
  const streaming = createCaller({ ...base, fetch: async () => new Response(new ReadableStream({ cancel() { cancelled = true; } })) });
  const handle = { id: 'test-id', operation: 'laptop.health', expiresAt: Date.now() + 60000, args: {} };
  await assert.rejects(streaming.read(handle), error => error.code === 'timeout' && error.handle === handle);
  assert.equal(cancelled, true);
});
