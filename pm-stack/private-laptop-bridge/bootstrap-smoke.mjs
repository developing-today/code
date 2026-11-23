#!/usr/bin/env node
// Create the smoke Worker metadata object only. No script/version/deployment,
// routes, domains, workers.dev setting changes, or settings updates are made.

export const API = 'https://api.cloudflare.com/client/v4';
export const ACCOUNT_SUFFIX = 'cbfa';
export const WORKER_NAME = 'private-laptop-bridge-smoke';
const PAGE_SIZE = 50;
const VERSION_PAGE_SIZE = 100;

export const CREATE_BODY = Object.freeze({
  name: WORKER_NAME,
  logpush: false,
  observability: Object.freeze({ enabled: false }),
  subdomain: Object.freeze({ enabled: false, previews_enabled: false }),
});

function fail(message) { throw new Error(message); }

export function validateInputs({ accountId, writeToken, readToken }) {
  if (!/^[a-f0-9]{32}$/i.test(accountId ?? '') || !accountId.toLowerCase().endsWith(ACCOUNT_SUFFIX)) {
    fail('bootstrap account ID is missing or not the approved account');
  }
  if (typeof writeToken !== 'string' || writeToken.length < 20) fail('bootstrap write credential is missing');
  if (typeof readToken !== 'string' || readToken.length < 20) fail('bootstrap read-only credential is missing');
}

function parseEnvelope(response, payload) {
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) fail('Cloudflare returned an invalid response');
  if (!response.ok || payload.success !== true) {
    const status = response.status;
    fail(status === 401 || status === 403
      ? `Cloudflare authorization denied (HTTP ${status})`
      : `Cloudflare API request failed (HTTP ${status})`);
  }
  if (!Object.hasOwn(payload, 'result')) fail('Cloudflare response omitted result');
  return payload.result;
}

async function requestWithInfo(fetchImpl, url, token, { method = 'GET', body, allowWorkerNotFound = false } = {}) {
  let response;
  try {
    response = await fetchImpl(url, {
      method,
      headers: {
        authorization: `Bearer ${token}`,
        accept: 'application/json',
        ...(body === undefined ? {} : { 'content-type': 'application/json' }),
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      redirect: 'error',
      signal: AbortSignal.timeout(15_000),
    });
  } catch {
    fail('Cloudflare API request failed before a verified response; do not retry creation automatically');
  }

  let payload;
  try { payload = await response.json(); } catch { fail(`Cloudflare returned invalid JSON (HTTP ${response.status})`); }
  if (response.status === 404 && allowWorkerNotFound) {
    if (payload?.success === false && Array.isArray(payload.errors) && payload.errors.length > 0 &&
        payload.errors.every(e => typeof e?.code === 'number' && /not found|does not exist|unknown worker/i.test(String(e.message)))) {
      return { absent: true, resultInfo: payload.result_info };
    }
  }
  return { result: parseEnvelope(response, payload), resultInfo: payload.result_info };
}

async function request(fetchImpl, accountId, token, path, options = {}) {
  return requestWithInfo(fetchImpl, `${API}/accounts/${encodeURIComponent(accountId)}${path}`, token, options);
}

function assertWorker(worker) {
  if (!worker || typeof worker !== 'object' || Array.isArray(worker)) fail('Worker readback is missing or malformed');
  if (worker.name !== WORKER_NAME || !/^[a-f0-9]{32}$/i.test(worker.id ?? '')) fail('Worker identity does not match the fixed smoke target');
  if (worker.logpush !== false || worker.observability?.enabled !== false) fail('Worker telemetry metadata is not explicitly disabled');
  if (worker.subdomain?.enabled !== false || worker.subdomain?.previews_enabled !== false) fail('Worker public subdomain or preview URL is enabled');
  if (!Object.hasOwn(worker, 'deployed_on') || worker.deployed_on !== null) fail('Worker already has or may have a deployment');
  if (!worker.references || !Array.isArray(worker.references.domains) || worker.references.domains.length !== 0) fail('Worker has a custom domain or domain state could not be verified');
  return worker;
}

async function listAll(fetchImpl, accountId, readToken, pathname, pageSize, params = {}) {
  const all = [];
  let expectedTotal;
  for (let page = 1; page <= 100; page += 1) {
    const query = new URLSearchParams({ ...params, page: String(page), per_page: String(pageSize) });
    const { result, resultInfo } = await request(fetchImpl, accountId, readToken, `${pathname}?${query}`);
    if (!Array.isArray(result)) fail('Cloudflare list response was not an array');
    all.push(...result);
    if (!resultInfo || !Number.isInteger(resultInfo.total_pages) || resultInfo.total_pages < 0 || resultInfo.total_pages > 100 ||
        !Number.isInteger(resultInfo.total_count) || resultInfo.total_count < 0) {
      fail('Cloudflare pagination metadata is missing or invalid');
    }
    if (expectedTotal === undefined) expectedTotal = resultInfo.total_count;
    if (resultInfo.total_count !== expectedTotal) fail('Cloudflare list changed during pagination');
    if (resultInfo.total_pages === 0 && page === 1 && expectedTotal === 0 && all.length === 0) return all;
    if (resultInfo.total_pages < page) fail('Cloudflare pagination metadata is missing or invalid');
    if (page >= resultInfo.total_pages) {
      if (all.length !== expectedTotal) fail('Cloudflare list did not return its complete item count');
      return all;
    }
  }
  fail('Cloudflare pagination exceeded the hard page bound');
}

async function listZones(fetchImpl, accountId, readToken) {
  const all = [];
  let expectedTotal;
  for (let page = 1; page <= 100; page += 1) {
    const query = new URLSearchParams({
      'account.id': accountId,
      type: 'full,partial,secondary,internal',
      page: String(page),
      per_page: String(PAGE_SIZE),
    });
    // Keep result_info for complete pagination proof.
    const url = `${API}/zones?${query}`;
    const { result, resultInfo } = await requestWithInfo(fetchImpl, url, readToken);
    if (!Array.isArray(result)) fail('Cloudflare zone response was not an array');
    all.push(...result);
    if (!resultInfo || !Number.isInteger(resultInfo.total_pages) || resultInfo.total_pages < 0 || resultInfo.total_pages > 100 ||
        !Number.isInteger(resultInfo.total_count) || resultInfo.total_count < 0) {
      fail('Cloudflare zone pagination metadata is missing or invalid');
    }
    if (expectedTotal === undefined) expectedTotal = resultInfo.total_count;
    if (resultInfo.total_count !== expectedTotal) fail('Cloudflare zone inventory changed during pagination');
    if (resultInfo.total_pages === 0 && page === 1 && expectedTotal === 0 && all.length === 0) return all;
    if (resultInfo.total_pages < page) fail('Cloudflare zone pagination metadata is missing or invalid');
    if (page >= resultInfo.total_pages) {
      if (all.length !== expectedTotal) fail('Cloudflare zone inventory is incomplete');
      return all;
    }
  }
  fail('Cloudflare zone pagination exceeded the hard page bound');
}

async function assertNoRoutes(fetchImpl, accountId, readToken) {
  const zones = await listZones(fetchImpl, accountId, readToken);
  for (const zone of zones) {
    if (typeof zone?.id !== 'string' || !zone.id) fail('Cloudflare zone inventory contains an invalid zone ID');
    const { result } = await requestWithInfo(fetchImpl, `${API}/zones/${encodeURIComponent(zone.id)}/workers/routes`, readToken);
    if (!Array.isArray(result)) fail('Cloudflare route response was not an array');
    if (result.some(route => route?.script === WORKER_NAME)) fail('A Cloudflare route already targets the smoke Worker');
  }
}

async function assertNoVersionsAndDeployments(fetchImpl, accountId, readToken, workerId) {
  const versions = await listAll(fetchImpl, accountId, readToken, `/workers/workers/${encodeURIComponent(workerId)}/versions`, VERSION_PAGE_SIZE);
  if (versions.length !== 0) fail('Smoke Worker already has a version; no upload or deployment was attempted');
  const { result, resultInfo } = await request(fetchImpl, accountId, readToken, `/workers/scripts/${encodeURIComponent(WORKER_NAME)}/deployments?page=1&per_page=100`);
  if (!result || !Array.isArray(result.deployments) || result.deployments.length !== 0 || resultInfo?.total_count !== 0) {
    fail('Smoke Worker deployment history is non-empty or unverifiable');
  }
}

async function readWorker(fetchImpl, accountId, readToken) {
  const response = await request(fetchImpl, accountId, readToken, `/workers/workers/${encodeURIComponent(WORKER_NAME)}`, { allowWorkerNotFound: true });
  return response.absent ? null : assertWorker(response.result);
}

export async function bootstrapSmoke({ accountId, writeToken, readToken, fetchImpl = fetch }) {
  validateInputs({ accountId, writeToken, readToken });

  const before = await readWorker(fetchImpl, accountId, readToken);
  if (before) {
    await assertNoRoutes(fetchImpl, accountId, readToken);
    await assertNoVersionsAndDeployments(fetchImpl, accountId, readToken, before.id);
    return 'already-present';
  }

  // The complete account route inventory is checked immediately before the
  // only write. Any read/scope/pagination error exits before creation.
  await assertNoRoutes(fetchImpl, accountId, readToken);
  const created = await request(fetchImpl, accountId, writeToken, '/workers/workers', { method: 'POST', body: CREATE_BODY });
  assertWorker(created.result);

  // Re-read using the read-only token, never the create credential.
  const after = await readWorker(fetchImpl, accountId, readToken);
  if (!after || after.id !== created.result.id) fail('Created Worker identity does not match read-only provider readback');
  await assertNoRoutes(fetchImpl, accountId, readToken);
  await assertNoVersionsAndDeployments(fetchImpl, accountId, readToken, after.id);
  return 'created';
}

if (process.argv[1] && new URL(import.meta.url).pathname === process.argv[1]) {
  try {
    const status = await bootstrapSmoke({
      accountId: process.env.CF_SMOKE_INIT_ACCOUNT_ID,
      writeToken: process.env.CF_SMOKE_INIT_WRITE_TOKEN,
      readToken: process.env.CF_SMOKE_INIT_READ_TOKEN,
    });
    console.log(`Smoke Worker init preflight passed (${status}); no code or deployment was uploaded.`);
  } catch (error) {
    console.error(error instanceof Error ? error.message : 'Smoke Worker initialization failed');
    process.exitCode = 1;
  }
}
