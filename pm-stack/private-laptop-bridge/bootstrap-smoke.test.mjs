import test from 'node:test';
import assert from 'node:assert/strict';
import { bootstrapSmoke, createInitReceipt, verifySmokeEmptyTarget, verifySmokePostUpload, CREATE_BODY, WORKER_NAME } from './bootstrap-smoke.mjs';

const accountId = `${'a'.repeat(28)}cbfa`;
const zoneId = '1'.repeat(32);
const expectedZoneIds = [zoneId];
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
  new Response(JSON.stringify({ success: true, result, ...(result_info === undefined ? {} : { result_info }) }), { status: 200 });
const okNoInfo = result => new Response(JSON.stringify({ success: true, result }), { status: 200 });
const missing = (code = 10007, message = 'Worker not found.') =>
  new Response(JSON.stringify({ success: false, errors: [{ code, message }] }), { status: 404 });

function fixture({ worker = null, createdWorker = safeWorker(), route = null, zones = [{ id: zoneId, account: { id: accountId } }], zoneResultInfo, zonePages, versions = [], versionResultInfo, deployments = [], deploymentResultInfo, deploymentNoInfo = false, createResult = null, authFailure = false, writeAuthFailure = false, missingCode = 10007, missingMessage = 'Worker not found.' } = {}) {
  const state = { worker, createdWorker, route, zones, zoneResultInfo, zonePages, versions, versionResultInfo, deployments, deploymentResultInfo, deploymentNoInfo, createResult, authFailure, writeAuthFailure, missingCode, missingMessage, calls: [] };
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
    if (parsed.pathname === '/client/v4/zones') {
      if (state.zonePages) {
        const page = Number(parsed.searchParams.get('page'));
        const result = state.zonePages[page - 1] ?? [];
        return ok(result, { total_pages: state.zonePages.length, page, per_page: 50, total_count: state.zonePages.flat().length, count: result.length });
      }
      return ok(state.zones, state.zoneResultInfo);
    }
    if (parsed.pathname.endsWith('/workers/routes')) return ok(state.route ? [state.route] : []);
    if (parsed.pathname.endsWith(`/workers/workers/${WORKER_NAME}`) && method === 'GET') {
      return state.worker ? ok(state.worker, undefined) : missing(state.missingCode, state.missingMessage);
    }
    if (parsed.pathname.endsWith('/workers/workers') && method === 'POST') {
      state.worker = state.createdWorker;
      return ok(state.createResult ?? state.createdWorker, undefined);
    }
    if (parsed.pathname.endsWith(`/workers/workers/${workerId}/versions`)) return ok(state.versions, state.versionResultInfo ?? { total_pages: 0, page: 1, per_page: 100, total_count: state.versions.length });
    if (parsed.pathname.endsWith(`/workers/scripts/${WORKER_NAME}/deployments`)) {
      if (state.deploymentNoInfo) return okNoInfo({ deployments: state.deployments });
      return ok({ deployments: state.deployments }, state.deploymentResultInfo ?? { total_pages: 1, page: 1, per_page: 100, total_count: state.deployments.length });
    }
    throw new Error(`unexpected synthetic request ${method} ${parsed.pathname}`);
  };
  return { state, fetchImpl };
}

function run({ fetchImpl, ...overrides }) {
  return bootstrapSmoke({ approvedAccountId: accountId, expectedZoneIds, writeToken, readToken, fetchImpl, ...overrides });
}

test('missing target creates exactly one metadata-only Worker and verifies exposure read-only', async () => {
  const { state, fetchImpl } = fixture();
  assert.deepEqual(await run({ fetchImpl }), { status: 'created', workerId });
  const writes = state.calls.filter(call => call.method !== 'GET');
  assert.equal(writes.length, 1);
  assert.equal(writes[0].method, 'POST');
  assert.equal(writes[0].token, `Bearer ${writeToken}`);
  assert.deepEqual(writes[0].body, CREATE_BODY);
  assert.ok(state.calls.filter(call => call.method === 'GET').every(call => call.token === `Bearer ${readToken}`));
  assert.equal(state.calls.some(call => call.url.pathname.includes('/scripts/') && call.method !== 'GET'), false);
});

test('same protected configuration is idempotent for an existing safe target', async () => {
  const { state, fetchImpl } = fixture({ worker: safeWorker() });
  assert.deepEqual(await run({ fetchImpl }), { status: 'already-present', workerId });
  assert.equal(state.calls.some(call => call.method === 'POST'), false);
});

test('full account identity and expected zone inventory are mandatory protected config', async () => {
  const { state, fetchImpl } = fixture();
  const wrongAccount = `${'b'.repeat(28)}beef`;
  await assert.rejects(run({ fetchImpl, approvedAccountId: wrongAccount }), error => {
    assert.match(error.message, /approved account configuration/);
    assert.equal(error.message.includes(wrongAccount), false);
    return true;
  });
  await assert.rejects(run({ fetchImpl, expectedZoneIds: undefined }), /approved zone inventory/);
  await assert.rejects(run({ fetchImpl, expectedZoneIds: 'not-json' }), /approved zone inventory/);
  assert.equal(state.calls.length, 0);
});

test('read and write credentials must be different before any request', async () => {
  const { state, fetchImpl } = fixture();
  await assert.rejects(run({ fetchImpl, readToken: writeToken }), /must be distinct/);
  assert.equal(state.calls.length, 0);
});

test('missing credentials fail before any network request', async () => {
  const { state, fetchImpl } = fixture();
  await assert.rejects(run({ fetchImpl, writeToken: '' }), /write credential is missing/);
  await assert.rejects(run({ fetchImpl, readToken: '' }), /read-only credential is missing/);
  assert.equal(state.calls.length, 0);
});

test('only the documented Worker-specific 404 means absent', async () => {
  const generic = fixture({ missingCode: 10000, missingMessage: 'Resource not found' });
  await assert.rejects(run({ fetchImpl: generic.fetchImpl }), /API request failed/);
  assert.equal(generic.state.calls.some(call => call.method === 'POST'), false);

  const wrongMessage = fixture({ missingCode: 10007, missingMessage: 'Not found' });
  await assert.rejects(run({ fetchImpl: wrongMessage.fetchImpl }), /API request failed/);
  assert.equal(wrongMessage.state.calls.some(call => call.method === 'POST'), false);
});

test('read and create authorization failures stop without retry', async () => {
  const deniedRead = fixture({ authFailure: true });
  await assert.rejects(run({ fetchImpl: deniedRead.fetchImpl }), /authorization denied/);
  assert.equal(deniedRead.state.calls.some(call => call.method === 'POST'), false);

  const deniedWrite = fixture({ writeAuthFailure: true });
  await assert.rejects(run({ fetchImpl: deniedWrite.fetchImpl }), /authorization denied/);
  assert.equal(deniedWrite.state.calls.filter(call => call.method === 'POST').length, 1);
});

test('zone scan requires owner, unique IDs and exact full-account inventory', async t => {
  const cases = [
    { zones: [{ id: zoneId, account: { id: 'f'.repeat(32) } }], expected: /owner or ID/ },
    { zones: [{ id: zoneId, account: { id: accountId } }, { id: zoneId, account: { id: accountId } }], expected: /duplicate ID/ },
      { zones: [], expected: /zone total count|does not match the approved/ },
      { zones: [{ id: zoneId, account: { id: accountId } }], zoneResultInfo: { total_pages: 1, page: 2, per_page: 50, total_count: 1 }, expected: /zone page does not match/ },
    { zones: [{ id: '2'.repeat(32), account: { id: accountId } }], expected: /does not match the approved/ },
  ];
  for (const item of cases) {
    await t.test(item.expected.source, async () => {
      const f = fixture({ zones: item.zones, zoneResultInfo: item.zoneResultInfo });
      await assert.rejects(run({ fetchImpl: f.fetchImpl }), item.expected);
      assert.equal(f.state.calls.some(call => call.method === 'POST'), false);
    });
  }
});

test('complete multi-page route inventory is compared to protected zone IDs', async () => {
  const ids = Array.from({ length: 51 }, (_, index) => index.toString(16).padStart(32, '0'));
  const zones = ids.map(id => ({ id, account: { id: accountId } }));
  const f = fixture({ zonePages: [zones.slice(0, 50), zones.slice(50)] });
  assert.deepEqual(await run({ fetchImpl: f.fetchImpl, expectedZoneIds: ids }), { status: 'created', workerId });
  const zoneRequests = f.state.calls.filter(call => call.url.pathname === '/client/v4/zones');
  assert.equal(zoneRequests.length, 4); // two complete scans: before and after create
});

test('known zero-zone inventory is accepted only when explicitly configured', async () => {
  const f = fixture({ zones: [], zoneResultInfo: { total_pages: 0, page: 1, per_page: 50, total_count: 0 } });
  assert.deepEqual(await bootstrapSmoke({ approvedAccountId: accountId, expectedZoneIds: [], writeToken, readToken, fetchImpl: f.fetchImpl }), { status: 'created', workerId });
  assert.equal(f.state.calls.some(call => call.method === 'POST'), true);
});

test('wrong target or unsafe existing Worker fails without edits', async t => {
  const wrong = safeWorker(); wrong.name = 'some-other-worker';
  const badTarget = fixture({ worker: wrong });
  await assert.rejects(run({ fetchImpl: badTarget.fetchImpl }), /identity does not match/);
  assert.equal(badTarget.state.calls.some(call => call.method === 'POST'), false);

  for (const change of [
    w => { w.subdomain.enabled = true; },
    w => { w.subdomain.previews_enabled = true; },
    w => { w.deployed_on = '2026-11-23T00:00:00Z'; },
    w => { w.references.domains = [{ hostname: 'example.invalid' }]; },
  ]) {
    const worker = safeWorker(); change(worker);
    const f = fixture({ worker });
    await t.test('unsafe Worker state', async () => {
      await assert.rejects(run({ fetchImpl: f.fetchImpl }));
      assert.equal(f.state.calls.some(call => call.method === 'POST'), false);
    });
  }
});

test('route conflict and incomplete pagination block creation', async () => {
  const routed = fixture({ route: { id: 'route', pattern: 'example.invalid/*', script: WORKER_NAME } });
  await assert.rejects(run({ fetchImpl: routed.fetchImpl }), /route already targets/);
  assert.equal(routed.state.calls.some(call => call.method === 'POST'), false);

  const incomplete = fixture({ zoneResultInfo: { total_pages: 1, page: 1, per_page: 50, total_count: 2 } });
  await assert.rejects(run({ fetchImpl: incomplete.fetchImpl }), /zone total count/);
  assert.equal(incomplete.state.calls.some(call => call.method === 'POST'), false);
});

test('create response identity mismatch stops after its single write', async () => {
  const mismatch = safeWorker(); mismatch.id = 'ffffffffffffffffffffffffffffffff';
  const f = fixture({ createResult: mismatch, createdWorker: safeWorker() });
  await assert.rejects(run({ fetchImpl: f.fetchImpl }), /Created Worker identity/);
  assert.equal(f.state.calls.filter(call => call.method === 'POST').length, 1);
});

test('empty deployment list is explicit evidence; missing list or conflicting metadata is denied', async t => {
  const empty = fixture({ worker: safeWorker(), deploymentNoInfo: true });
  assert.deepEqual(await run({ fetchImpl: empty.fetchImpl }), { status: 'already-present', workerId });

  for (const response of [
    { result: {}, result_info: { total_count: 0 } },
    { result: { deployments: [] }, result_info: { total_count: 1 } },
    { result: { deployments: [{ id: 'deployed' }] }, result_info: undefined },
  ]) {
    await t.test('non-empty or unverifiable deployment history', async () => {
      const f = fixture({ worker: safeWorker() });
      const original = f.fetchImpl;
      f.fetchImpl = async (url, init) => {
        if (new URL(url).pathname.endsWith(`/workers/scripts/${WORKER_NAME}/deployments`)) {
          return response.result_info === undefined ? okNoInfo(response.result) : ok(response.result, response.result_info);
        }
        return original(url, init);
      };
      await assert.rejects(run({ fetchImpl: f.fetchImpl }), /deployment history/);
      assert.equal(f.state.calls.some(call => call.method === 'POST'), false);
    });
  }
});

test('zero versions and deployment history are required', async () => {
  const { state, fetchImpl } = fixture({ worker: safeWorker() });
  assert.deepEqual(await run({ fetchImpl }), { status: 'already-present', workerId });
  assert.ok(state.calls.some(call => call.url.pathname.endsWith('/versions')));
  assert.ok(state.calls.some(call => call.url.pathname.endsWith('/deployments')));

  const withVersion = fixture({ worker: safeWorker(), versions: [{ id: 'synthetic-version' }] });
  await assert.rejects(run({ fetchImpl: withVersion.fetchImpl }), /already has a version/);
  assert.equal(withVersion.state.calls.some(call => call.method === 'POST'), false);
});

test('read-only upload preflight binds the init Worker ID and postflight keeps it private', async () => {
  const f = fixture({ worker: safeWorker() });
  assert.deepEqual(await verifySmokeEmptyTarget({ approvedAccountId: accountId, expectedZoneIds, readToken, expectedWorkerId: workerId, fetchImpl: f.fetchImpl }), { workerId, workerName: WORKER_NAME });
  assert.deepEqual(await verifySmokePostUpload({ approvedAccountId: accountId, expectedZoneIds, readToken, expectedWorkerId: workerId, fetchImpl: f.fetchImpl }), { workerId, workerName: WORKER_NAME });
  assert.equal(f.state.calls.some(call => call.method !== 'GET'), false);

  await assert.rejects(verifySmokeEmptyTarget({ approvedAccountId: accountId, expectedZoneIds, readToken, expectedWorkerId: 'f'.repeat(32), fetchImpl: f.fetchImpl }), /differs from the initialization receipt/);
});

test('init receipt binds exact trusted run, account hash and Worker without storing account ID', () => {
  const receipt = createInitReceipt({
    approvedAccountId: accountId, workerId, status: 'created', repository: 'developing-today/code',
    sourceSha: 'a'.repeat(40), runId: '123456789', runAttempt: 1, event: 'workflow_dispatch', branch: 'main',
  });
  assert.equal(receipt.worker_name, WORKER_NAME);
  assert.equal(receipt.worker_id, workerId);
  assert.equal(receipt.source_sha, 'a'.repeat(40));
  assert.equal(receipt.account_id_sha256.length, 64);
  assert.equal(JSON.stringify(receipt).includes(accountId), false);
  assert.throws(() => createInitReceipt({ approvedAccountId: accountId, workerId, status: 'created', repository: 'developing.today/code', sourceSha: 'a'.repeat(40), runId: '123456789', runAttempt: 1, event: 'workflow_dispatch', branch: 'main' }), /trusted workflow run metadata/);
});
