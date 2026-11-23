import test from 'node:test';
import assert from 'node:assert/strict';
import { bootstrapSmoke, CREATE_BODY, WORKER_NAME } from './bootstrap-smoke.mjs';

const accountId = `${'a'.repeat(28)}cbfa`;
const writeToken = 'synthetic-write-token-000000';
const readToken = 'synthetic-read-token-0000000';
const workerId = '0123456789abcdef0123456789abcdef';
const safeWorker = () => ({
  id: workerId,
  name: WORKER_NAME,
  logpush: false,
  observability: { enabled: false },
  subdomain: { enabled: false, previews_enabled: false },
  deployed_on: null,
  references: { domains: [] },
});
const ok = (result, result_info = { total_pages: 1, page: 1, per_page: 50, total_count: Array.isArray(result) ? result.length : 0 }) =>
  new Response(JSON.stringify({ success: true, result, result_info }), { status: 200 });
const missing = () => new Response(JSON.stringify({ success: false, errors: [{ code: 10007, message: 'Worker not found' }] }), { status: 404 });

function fixture({ worker = null, createdWorker = safeWorker(), route = null, zones = [{ id: 'z'.repeat(32) }], zoneResultInfo, versions = [], versionResultInfo, deployments = [], createResult = null, authFailure = false, writeAuthFailure = false } = {}) {
  const state = { worker, createdWorker, route, zones, zoneResultInfo, versions, versionResultInfo, deployments, createResult, authFailure, writeAuthFailure, calls: [] };
  const fetchImpl = async (url, init = {}) => {
    const parsed = new URL(url);
    const method = init.method ?? 'GET';
    const token = init.headers?.authorization;
    state.calls.push({ url: parsed, method, token, body: init.body ? JSON.parse(init.body) : undefined });
    if (state.authFailure && token === `Bearer ${readToken}`) {
      return new Response(JSON.stringify({ success: false, errors: [{ code: 10000, message: 'Authentication error' }] }), { status: 403 });
    }
    if (state.writeAuthFailure && method === 'POST' && token === `Bearer ${writeToken}`) {
      return new Response(JSON.stringify({ success: false, errors: [{ code: 10000, message: 'Authentication error' }] }), { status: 403 });
    }
    if (parsed.pathname === '/client/v4/zones') return ok(state.zones, state.zoneResultInfo ?? undefined);
    if (parsed.pathname.endsWith('/workers/routes')) return ok(state.route ? [state.route] : []);
    if (parsed.pathname.endsWith(`/workers/workers/${WORKER_NAME}`) && method === 'GET') {
      return state.worker ? ok(state.worker, undefined) : missing();
    }
    if (parsed.pathname.endsWith('/workers/workers') && method === 'POST') {
      state.worker = state.createdWorker;
      return ok(state.createResult ?? state.createdWorker, undefined);
    }
    if (parsed.pathname.endsWith(`/workers/workers/${workerId}/versions`)) return ok(state.versions, state.versionResultInfo ?? undefined);
    if (parsed.pathname.endsWith(`/workers/scripts/${WORKER_NAME}/deployments`)) {
      return ok({ deployments: state.deployments }, { total_pages: 1, page: 1, per_page: 100, total_count: state.deployments.length });
    }
    throw new Error(`unexpected synthetic request ${method} ${parsed.pathname}`);
  };
  return { state, fetchImpl };
}

test('missing target gets exactly one metadata-only create, then read-only no-exposure proof', async () => {
  const { state, fetchImpl } = fixture();
  assert.equal(await bootstrapSmoke({ accountId, writeToken, readToken, fetchImpl }), 'created');
  const writes = state.calls.filter(call => call.method !== 'GET');
  assert.equal(writes.length, 1);
  assert.equal(writes[0].method, 'POST');
  assert.equal(writes[0].token, `Bearer ${writeToken}`);
  assert.deepEqual(writes[0].body, CREATE_BODY);
  assert.ok(state.calls.filter(call => call.method === 'GET').every(call => call.token === `Bearer ${readToken}`));
  assert.equal(state.calls.some(call => call.url.pathname.includes('/scripts/') && call.method !== 'GET'), false);
});

test('provider zero-page pagination is accepted only with an explicit zero item count', async () => {
  const { state, fetchImpl } = fixture({
    zones: [],
    zoneResultInfo: { total_pages: 0, page: 1, per_page: 50, total_count: 0 },
  });
  assert.equal(await bootstrapSmoke({ accountId, writeToken, readToken, fetchImpl }), 'created');
  assert.equal(state.calls.some(call => call.method === 'POST'), true);
});

test('already existing safe target is idempotent and is never overwritten', async () => {
  const { state, fetchImpl } = fixture({ worker: safeWorker() });
  assert.equal(await bootstrapSmoke({ accountId, writeToken, readToken, fetchImpl }), 'already-present');
  assert.equal(state.calls.some(call => call.method === 'POST'), false);
});

test('wrong account suffix and wrong returned target fail closed', async () => {
  const { state, fetchImpl } = fixture();
  await assert.rejects(bootstrapSmoke({ accountId: `${'b'.repeat(28)}beef`, writeToken, readToken, fetchImpl }), /not the approved account/);
  assert.equal(state.calls.length, 0);

  const wrong = safeWorker(); wrong.name = 'some-other-worker';
  const existing = fixture({ worker: wrong });
  await assert.rejects(bootstrapSmoke({ accountId, writeToken, readToken, fetchImpl: existing.fetchImpl }), /identity does not match/);
  assert.equal(existing.state.calls.some(call => call.method === 'POST'), false);
});

test('read authorization failure is not mistaken for an absent Worker', async () => {
  const { state, fetchImpl } = fixture({ authFailure: true });
  await assert.rejects(bootstrapSmoke({ accountId, writeToken, readToken, fetchImpl }), /authorization denied/);
  assert.equal(state.calls.some(call => call.method === 'POST'), false);
});

test('create authorization failure is bounded to one denied create attempt', async () => {
  const { state, fetchImpl } = fixture({ writeAuthFailure: true });
  await assert.rejects(bootstrapSmoke({ accountId, writeToken, readToken, fetchImpl }), /authorization denied/);
  assert.equal(state.calls.filter(call => call.method === 'POST').length, 1);
  assert.equal(state.calls.filter(call => call.method !== 'GET').length, 1);
});

test('pre-existing public Worker fails closed without edits', async t => {
  for (const change of [
    w => { w.subdomain.enabled = true; },
    w => { w.subdomain.previews_enabled = true; },
    w => { w.deployed_on = '2026-11-23T00:00:00Z'; },
    w => { w.references.domains = [{ hostname: 'example.invalid' }]; },
  ]) {
    const worker = safeWorker(); change(worker);
    const { state, fetchImpl } = fixture({ worker });
    await t.test('unsafe existing state', async () => {
      await assert.rejects(bootstrapSmoke({ accountId, writeToken, readToken, fetchImpl }));
      assert.equal(state.calls.some(call => call.method === 'POST'), false);
    });
  }
});

test('a route already pointing at the fixed name blocks creation', async () => {
  const { state, fetchImpl } = fixture({ zones: [{ id: 'z'.repeat(32) }], route: { id: 'synthetic-route', pattern: 'example.invalid/*', script: WORKER_NAME } });
  await assert.rejects(bootstrapSmoke({ accountId, writeToken, readToken, fetchImpl }), /route already targets/);
  assert.equal(state.calls.some(call => call.method === 'POST'), false);
});

test('incomplete account route inventory blocks creation', async () => {
  const { state, fetchImpl } = fixture({ zoneResultInfo: { total_pages: 1, page: 1, per_page: 50, total_count: 2 } });
  await assert.rejects(bootstrapSmoke({ accountId, writeToken, readToken, fetchImpl }), /inventory is incomplete/);
  assert.equal(state.calls.some(call => call.method === 'POST'), false);
});

test('create response identity mismatch fails without follow-up mutation', async () => {
  const mismatch = safeWorker(); mismatch.id = 'ffffffffffffffffffffffffffffffff';
  const { state, fetchImpl } = fixture({ createResult: mismatch, createdWorker: safeWorker() });
  await assert.rejects(bootstrapSmoke({ accountId, writeToken, readToken, fetchImpl }), /Created Worker identity/);
  assert.equal(state.calls.filter(call => call.method === 'POST').length, 1);
});

test('zero versions and zero deployments are required; any deployment blocks', async t => {
  const { state, fetchImpl } = fixture({ worker: safeWorker() });
  assert.equal(await bootstrapSmoke({ accountId, writeToken, readToken, fetchImpl }), 'already-present');
  assert.ok(state.calls.some(call => call.url.pathname.endsWith('/versions')));
  assert.ok(state.calls.some(call => call.url.pathname.endsWith('/deployments')));

  const deployed = fixture({ worker: safeWorker(), deployments: [{ id: 'synthetic-deployment' }] });
  await t.test('non-empty deployment history is denied', async () => {
    await assert.rejects(bootstrapSmoke({ accountId, writeToken, readToken, fetchImpl: deployed.fetchImpl }), /deployment history/);
    assert.equal(deployed.state.calls.some(call => call.method === 'POST'), false);
  });
});

test('missing read-only credential fails before any API call', async () => {
  const { state, fetchImpl } = fixture();
  await assert.rejects(bootstrapSmoke({ accountId, writeToken, readToken: '', fetchImpl }), /read-only credential is missing/);
  assert.equal(state.calls.length, 0);
});
