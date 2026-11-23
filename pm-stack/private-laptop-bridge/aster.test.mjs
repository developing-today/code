// Oracle: complete CLI grammar, fixed operation dispatch, and only sanitized
// accepted/result/error records. All dependencies are synthetic; no live access.
import test from 'node:test';
import assert from 'node:assert/strict';
import { parseAsterArgs, runAster } from './aster.mjs';

const configPath = '/owner/caller.json';
const argv = (...args) => ['--config', configPath, ...args];
const counts = { queued: 1, claimed: 0, parked: 0, handed_off: 0, blocked: 0, done: 0, cancelled: 0, failed: 0 };
const samples = {
  'fleet.queue.list': { ok: true, data: { tasks: [{ id: 1, status: 'queued' }], complete: false, total: null, fetchedAt: Date.now() } },
  'fleet.queue.health': { ok: true, data: { ...counts, fetchedAt: Date.now() } },
  'laptop.health': { ok: true, data: { queueBroker: 'up' } },
  'laptop.diagnostics': { ok: true, data: { uptimeSeconds: 123, load1: 0.25, memoryFreeBytes: 1024 } },
};
function fixtures(overrides = {}) {
  const stdout = [], stderr = [], activity = [];
  const config = { origin: 'https://laptop-bridge-staging.security.cab', audience: 'aster-test', principal: 'aster-caller', privateKey: { sensitive: 'never-print' },
    accessClientId:'synthetic-caller-client',accessClientSecret:'synthetic-access-secret-that-is-never-live' };
  const methods = { fleetQueueList: 'fleet.queue.list', fleetQueueHealth: 'fleet.queue.health', laptopHealth: 'laptop.health', laptopDiagnostics: 'laptop.diagnostics' };
  const caller = Object.fromEntries(Object.entries(methods).map(([method, operation]) => [method, async (...args) => {
    activity.push({ method, args });
    return { id: 'read-1', operation, expiresAt: Date.now() + 60000, privateTimingState: 'never-print' };
  }]));
  caller.wait = async (handle, options) => { activity.push({ method: 'wait', handle, options }); return { status: 'complete', handle, result: samples[handle.operation] }; };
  const deps = {
    getuid: () => 1000,
    loadConfig: async (path, getuid) => { activity.push({ method: 'config', path }); assert.equal(getuid(), 1000); return config; },
    importKey: async key => { assert.equal(key, config.privateKey); activity.push({ method: 'import' }); return 'in-memory-CryptoKey'; },
    callerFactory: options => { activity.push({ method: 'factory', options }); return caller; },
    stdout: text => stdout.push(text), stderr: text => stderr.push(text), ...overrides,
  };
  return { deps, stdout, stderr, activity, caller };
}

test('Aster: entire grammar is strict and malformed arguments never read config or network', async () => {
  const malformed = [[], ['queue-health'], ['--help','extra'], argv('queue-list'), argv('queue-list','--limit'),
    argv('queue-list','--limit','0'), argv('queue-list','--limit','21'), argv('queue-list','--limit','01'),
    argv('queue-list','--limit','1e1'), argv('queue-list','--limit=1'), argv('queue-list','--limit','1','--status','unknown'),
    argv('queue-list','--limit','1','--status','queued','--limit','2'), argv('queue-health','--url','http://local'),
    argv('laptop-health','stray'), argv('shell','echo','hi'), ['--config','relative','laptop-health'],
    ['--config','/owner/\0bad','laptop-health'], argv('constructor'), argv('__proto__'),
  ];
  for (const args of malformed) {
    const f = fixtures();
    assert.throws(() => parseAsterArgs(args), /usage/);
    assert.equal(await runAster(args, f.deps), 2);
    assert.deepEqual(f.activity, []);
    assert.deepEqual(f.stdout, []);
  }
  assert.deepEqual(parseAsterArgs(argv('queue-list','--limit','20','--status','queued')).args, { limit: 20, status: 'queued' });
});

test('Aster: root refusal precedes config reads, help has no data access', async () => {
  const f = fixtures({ getuid: () => 0 });
  assert.equal(await runAster(argv('laptop-health'), f.deps), 1);
  assert.deepEqual(f.activity, []);
  assert.deepEqual(f.stderr.map(JSON.parse), [{ event: 'error', error: 'non-root-required' }]);
  const help = fixtures();
  assert.equal(await runAster(['--help'], help.deps), 0);
  assert.deepEqual(help.activity, []);
});

test('Aster: four commands dispatch named reads and emit bounded projected records only', async () => {
  const commands = [['queue-list','--limit','1','--status','queued'], ['queue-health'], ['laptop-health'], ['laptop-diagnostics']];
  for (const command of commands) {
    const f = fixtures();
    assert.equal(await runAster(argv(...command), f.deps), 0);
    assert.deepEqual(f.stderr, []);
    const records = f.stdout.map(JSON.parse);
    assert.equal(records.length, 2);
    assert.equal(records[0].event, 'accepted');
    assert.equal(records[1].event, 'complete');
    assert.deepEqual(records[1].result, samples[records[1].operation]);
    assert.equal(f.stdout.join('').includes('never-print'), false);
    assert.ok(f.stdout.every(line => Buffer.byteLength(line) <= 8192));
    const factory = f.activity.find(a => a.method === 'factory');
    assert.equal(factory.options.device, 'amd');
    assert.equal(factory.options.privateKey, 'in-memory-CryptoKey');
    const wait = f.activity.find(a => a.method === 'wait');
    assert.equal(wait.handle.privateTimingState, 'never-print', 'original SDK timing handle remains internal');
    assert.ok(wait.options.signal instanceof AbortSignal);
  }
});

test('Aster: uncertain submit stops once without waiting or retrying', async () => {
  const f = fixtures();
  let submits = 0;
  f.caller.laptopHealth = async () => { submits++; throw Object.assign(new Error('sensitive transport message'), { code: 'submit-outcome-unknown' }); };
  assert.equal(await runAster(argv('laptop-health'), f.deps), 1);
  assert.equal(submits, 1);
  assert.deepEqual(f.stdout, []);
  assert.equal(f.activity.some(a => a.method === 'wait'), false);
  assert.deepEqual(f.stderr.map(JSON.parse), [{ event: 'error', error: 'submit-outcome-unknown' }]);
});

test('Aster: read failure retains accepted receipt but never exposes raw errors', async () => {
  const f = fixtures();
  f.caller.wait = async () => { throw new Error('secret network details'); };
  assert.equal(await runAster(argv('laptop-health'), f.deps), 1);
  assert.equal(f.stdout.map(JSON.parse)[0].event, 'accepted');
  assert.equal(f.stdout.length, 1);
  assert.deepEqual(f.stderr.map(JSON.parse), [{ event: 'error', error: 'unavailable' }]);
});

test('Aster: malformed or excessive output fails before disclosure; unavailable stays a failure', async () => {
  for (const result of [{ ok: true, data: { queueBroker: 'up', token: 'secret' } },
    { ok: true, data: { queueBroker: 'secret'.repeat(3000) } }]) {
    const f = fixtures();
    f.caller.wait = async () => ({ status: 'complete', result });
    assert.equal(await runAster(argv('laptop-health'), f.deps), 1);
    assert.equal(f.stdout.length, 1);
    assert.equal(f.stdout.join('').includes('secret'), false);
    assert.deepEqual(f.stderr.map(JSON.parse), [{ event: 'error', error: 'invalid-response' }]);
  }
  const f = fixtures();
  f.caller.wait = async () => ({ status: 'complete', result: { ok: false, error: 'unavailable' } });
  assert.equal(await runAster(argv('laptop-health'), f.deps), 1);
  assert.deepEqual(f.stdout.map(JSON.parse)[1].result, { ok: false, error: 'unavailable' });
});

test('Aster: real CLI -> signed caller SDK -> workerd -> device -> projected output', async t => {
  const { Miniflare } = await import('miniflare');
  const { createCaller } = await import('./caller.mjs');
  const { createClient } = await import('./client.mjs');
  const { b64url, operations } = await import('./protocol.mjs');
  const { fileURLToPath } = await import('node:url');
  const origin = 'https://laptop-bridge-staging.security.cab', audience = 'aster-offline';
  const keys = {}, principals = [];
  const at = Date.now();
  for (const [id, role] of [['aster','caller'], ['amd-device','device']]) {
    keys[id] = await crypto.subtle.generateKey('Ed25519', true, ['sign','verify']);
    principals.push({ id, role, device: 'amd', publicKey: b64url(await crypto.subtle.exportKey('raw', keys[id].publicKey)),accessClientId:id==='aster'?'synthetic-caller-client':'synthetic-device-client',
      issuedAt: at - 1000, expiresAt: at + 120000, revoked: false, scopes: [...operations] });
  }
  const mf = new Miniflare({ name: 'aster-cli-offline', host: '127.0.0.1', modules: true,
    scriptPath: fileURLToPath(new URL('./runtime-test-entry.mjs', import.meta.url)), modulesRules: [{ type: 'ESModule', include: ['**/*.mjs'] }],
    compatibilityDate: '2026-07-30', bindings: { ENABLED: 'true', BRIDGE_REGISTRY: JSON.stringify({ audience, principals }) },
    durableObjects: { BRIDGE: { className: 'PrivateLaptopBridge', useSQLite: true } }, durableObjectsPersist: false,
    outboundService: () => new Response('offline only', { status: 599 }),
  });
  t.after(() => mf.dispose());
  await mf.ready;
  const dispatch = async (request, init) => {
    assert.equal(init.redirect, 'error');
    return mf.dispatchFetch(request.url, { method: request.method, headers: Object.fromEntries(request.headers), body: await request.text() });
  };
  const device = createClient({ origin, audience, principal: 'amd-device', privateKey: keys['amd-device'].privateKey,
    accessClientId:'synthetic-device-client',accessClientSecret:'synthetic-access-secret-that-is-never-live',
    getuid: () => 1000, fetch: dispatch, execute: async operation => samples[operation] });
  const jwk = await crypto.subtle.exportKey('jwk', keys.aster.privateKey);
  const config = { origin, audience, principal: 'aster', privateKey: Object.fromEntries(['kty','crv','x','d'].map(k => [k,jwk[k]])),
    accessClientId:'synthetic-caller-client',accessClientSecret:'synthetic-access-secret-that-is-never-live' };
  const commands = [['queue-list','--limit','1','--status','queued'], ['queue-health'], ['laptop-health'], ['laptop-diagnostics']];
  for (const command of commands) {
    const stdout = [], stderr = [];
    const code = await runAster(argv(...command), { getuid: () => 1000, loadConfig: async () => config,
      callerFactory: options => {
        const caller = createCaller({ ...options, fetch: dispatch });
        return { ...caller, wait: async (handle, waitOptions) => { assert.equal(await device.step(), true); return caller.wait(handle, waitOptions); } };
      }, stdout: text => stdout.push(text), stderr: text => stderr.push(text),
    });
    assert.equal(code, 0, stderr.join(''));
    assert.deepEqual(stderr, []);
    const [accepted, complete] = stdout.map(JSON.parse);
    assert.equal(accepted.event, 'accepted');
    assert.equal(complete.id, accepted.handle.id);
    assert.deepEqual(complete.result, samples[complete.operation]);
    assert.equal(stdout.join('').includes(jwk.d), false);
  }
});
