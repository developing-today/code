#!/usr/bin/env node
// Create the smoke Worker metadata object only. No script/version/deployment,
// routes, domains, workers.dev setting changes, or settings updates are made.
import { createHash } from 'node:crypto';
import { writeFile } from 'node:fs/promises';

export const API = 'https://api.cloudflare.com/client/v4';
export const ACCOUNT_SUFFIX = 'cbfa';
export const WORKER_NAME = 'private-laptop-bridge-smoke';
export const INIT_WORKFLOW = '.github/workflows/private-laptop-bridge-smoke-init.yml';
const PAGE_SIZE = 50;
const VERSION_PAGE_SIZE = 100;

export const CREATE_BODY = Object.freeze({
  name: WORKER_NAME,
  logpush: false,
  observability: Object.freeze({ enabled: false }),
  subdomain: Object.freeze({ enabled: false, previews_enabled: false }),
});

function fail(message) { throw new Error(message); }

function parseExpectedZoneIds(value) {
  let ids = value;
  if (typeof ids === 'string') {
    try { ids = JSON.parse(ids); } catch { fail('approved zone inventory configuration is invalid'); }
  }
  if (!Array.isArray(ids) || ids.some(id => typeof id !== 'string' || !/^[a-f0-9]{32}$/i.test(id))) {
    fail('approved zone inventory configuration is missing or invalid');
  }
  if (new Set(ids.map(id => id.toLowerCase())).size !== ids.length) fail('approved zone inventory contains duplicate IDs');
  return ids.map(id => id.toLowerCase()).sort();
}

export function validateInputs({ approvedAccountId, writeToken, readToken, expectedZoneIds }) {
  if (!/^[a-f0-9]{32}$/i.test(approvedAccountId ?? '') || !approvedAccountId.toLowerCase().endsWith(ACCOUNT_SUFFIX)) {
    fail('approved account configuration is missing or invalid');
  }
  if (typeof writeToken !== 'string' || writeToken.length < 20) fail('bootstrap write credential is missing');
  if (typeof readToken !== 'string' || readToken.length < 20) fail('bootstrap read-only credential is missing');
  if (writeToken === readToken) fail('bootstrap read and write credentials must be distinct');
  return { approvedAccountId, expectedZoneIds: parseExpectedZoneIds(expectedZoneIds) };
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
    if (payload?.success === false && Array.isArray(payload.errors) && payload.errors.length === 1 &&
        payload.errors[0]?.code === 10007 && /^worker not found\.?$/i.test(String(payload.errors[0].message))) {
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
  let expectedPages;
  for (let page = 1; page <= 100; page += 1) {
    const query = new URLSearchParams({ ...params, page: String(page), per_page: String(pageSize) });
    const { result, resultInfo } = await request(fetchImpl, accountId, readToken, `${pathname}?${query}`);
    if (!Array.isArray(result)) fail('Cloudflare list response was not an array');
    all.push(...result);
    if (resultInfo !== undefined) {
      if (!resultInfo || typeof resultInfo !== 'object' || Array.isArray(resultInfo)) fail('Cloudflare pagination metadata is invalid');
      if (resultInfo.page !== undefined && resultInfo.page !== page) fail('Cloudflare pagination page does not match the request');
      if (resultInfo.per_page !== undefined && resultInfo.per_page !== pageSize) fail('Cloudflare pagination size does not match the request');
      if (resultInfo.count !== undefined && resultInfo.count !== result.length) fail('Cloudflare pagination item count is inconsistent');
      if (resultInfo.total_count !== undefined) {
        if (!Number.isInteger(resultInfo.total_count) || resultInfo.total_count < 0) fail('Cloudflare pagination total count is invalid');
        if (expectedTotal === undefined) expectedTotal = resultInfo.total_count;
        if (resultInfo.total_count !== expectedTotal) fail('Cloudflare list changed during pagination');
      }
      if (resultInfo.total_pages !== undefined) {
        if (!Number.isInteger(resultInfo.total_pages) || resultInfo.total_pages < 0 || resultInfo.total_pages > 100) fail('Cloudflare pagination total pages is invalid');
        if (expectedPages === undefined) expectedPages = resultInfo.total_pages;
        if (resultInfo.total_pages !== expectedPages) fail('Cloudflare list changed during pagination');
      }
    }
    if (expectedTotal !== undefined && all.length > expectedTotal) fail('Cloudflare list exceeded its reported total count');
    const completeByPages = expectedPages !== undefined && (expectedPages === 0 ? page === 1 && all.length === 0 : page >= expectedPages);
    const completeByCount = expectedTotal !== undefined && all.length === expectedTotal;
    const completeByShortPage = result.length < pageSize;
    if (completeByPages || completeByCount || (expectedPages === undefined && expectedTotal === undefined && completeByShortPage)) {
      if (expectedTotal !== undefined && all.length !== expectedTotal) fail('Cloudflare list did not return its complete item count');
      return all;
    }
    if (result.length < pageSize && !completeByCount) fail('Cloudflare list ended before its reported total count');
  }
  fail('Cloudflare pagination exceeded the hard page bound');
}

async function listZones(fetchImpl, accountId, readToken, expectedZoneIds) {
  const all = [];
  let expectedTotal;
  let expectedPages;
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
    if (result.length > PAGE_SIZE) fail('Cloudflare zone page exceeded the requested bound');
    const pageIds = new Set(all.map(zone => zone.id.toLowerCase()));
    for (const zone of result) {
      if (typeof zone?.id !== 'string' || !/^[a-f0-9]{32}$/i.test(zone.id) || zone?.account?.id?.toLowerCase() !== accountId.toLowerCase()) {
        fail('Cloudflare zone inventory contains an invalid owner or ID');
      }
      const id = zone.id.toLowerCase();
      if (pageIds.has(id)) {
        fail('Cloudflare zone inventory contains a duplicate ID');
      }
      pageIds.add(id);
    }
    all.push(...result);
    if (resultInfo !== undefined) {
      if (!resultInfo || typeof resultInfo !== 'object' || Array.isArray(resultInfo)) fail('Cloudflare zone pagination metadata is invalid');
      if (resultInfo.page !== undefined && resultInfo.page !== page) fail('Cloudflare zone page does not match the request');
      if (resultInfo.per_page !== undefined && resultInfo.per_page !== PAGE_SIZE) fail('Cloudflare zone page size does not match the request');
      if (resultInfo.count !== undefined && resultInfo.count !== result.length) fail('Cloudflare zone page count is inconsistent');
      if (resultInfo.total_count !== undefined) {
        if (!Number.isInteger(resultInfo.total_count) || resultInfo.total_count !== expectedZoneIds.length) fail('Cloudflare zone total count differs from approved inventory');
        if (expectedTotal === undefined) expectedTotal = resultInfo.total_count;
        if (resultInfo.total_count !== expectedTotal) fail('Cloudflare zone inventory changed during pagination');
      }
      if (resultInfo.total_pages !== undefined) {
        if (!Number.isInteger(resultInfo.total_pages) || resultInfo.total_pages < 0 || resultInfo.total_pages > 100) fail('Cloudflare zone total pages is invalid');
        if (resultInfo.total_pages > 0 && expectedZoneIds.length > 0 && resultInfo.total_pages !== Math.ceil(expectedZoneIds.length / PAGE_SIZE)) fail('Cloudflare zone total pages differs from approved inventory');
        if (expectedPages === undefined) expectedPages = resultInfo.total_pages;
        if (resultInfo.total_pages !== expectedPages) fail('Cloudflare zone inventory changed during pagination');
      }
    }
    if (all.length > expectedZoneIds.length) fail('Cloudflare zone inventory exceeds approved inventory');
    const completeByCount = all.length === expectedZoneIds.length;
    const completeByPages = expectedPages !== undefined && (expectedPages === 0 ? page === 1 && all.length === 0 : page >= expectedPages);
    if (completeByCount && (expectedPages === undefined || completeByPages)) return all;
    if (result.length < PAGE_SIZE && !completeByCount) fail('Cloudflare zone inventory ended before all approved zones were returned');
    if (expectedPages !== undefined && page >= expectedPages && !completeByCount) fail('Cloudflare zone inventory is incomplete');
  }
  fail('Cloudflare zone pagination exceeded the hard page bound');
}

async function assertNoRoutes(fetchImpl, accountId, readToken, expectedZoneIds) {
  const zones = await listZones(fetchImpl, accountId, readToken, expectedZoneIds);
  const seen = new Set();
  for (const zone of zones) {
    if (typeof zone?.id !== 'string' || !/^[a-f0-9]{32}$/i.test(zone.id) || zone?.account?.id?.toLowerCase() !== accountId.toLowerCase()) {
      fail('Cloudflare zone inventory contains an invalid owner or ID');
    }
    const zoneId = zone.id.toLowerCase();
    if (seen.has(zoneId)) fail('Cloudflare zone inventory contains a duplicate ID');
    seen.add(zoneId);
    const { result } = await requestWithInfo(fetchImpl, `${API}/zones/${encodeURIComponent(zone.id)}/workers/routes`, readToken);
    if (!Array.isArray(result)) fail('Cloudflare route response was not an array');
    if (result.some(route => route?.script === WORKER_NAME)) fail('A Cloudflare route already targets the smoke Worker');
  }
  const actualIds = [...seen].sort();
  if (actualIds.length !== expectedZoneIds.length || actualIds.some((id, index) => id !== expectedZoneIds[index])) {
    fail('Cloudflare zone inventory does not match the approved full-account inventory');
  }
}

async function assertNoVersionsAndDeployments(fetchImpl, accountId, readToken, workerId) {
  const versions = await listAll(fetchImpl, accountId, readToken, `/workers/workers/${encodeURIComponent(workerId)}/versions`, VERSION_PAGE_SIZE);
  if (versions.length !== 0) fail('Smoke Worker already has a version; no upload or deployment was attempted');
  await assertNoDeployments(fetchImpl, accountId, readToken);
}

async function assertNoDeployments(fetchImpl, accountId, readToken) {
  const { result, resultInfo } = await request(fetchImpl, accountId, readToken, `/workers/scripts/${encodeURIComponent(WORKER_NAME)}/deployments?page=1&per_page=100`);
  if (!result || typeof result !== 'object' || Array.isArray(result) || !Object.hasOwn(result, 'deployments') ||
      !Array.isArray(result.deployments) || result.deployments.length !== 0 ||
      (resultInfo?.total_count !== undefined && resultInfo.total_count !== 0)) {
    fail('Smoke Worker deployment history is non-empty or unverifiable');
  }
}

async function readWorker(fetchImpl, accountId, readToken) {
  const response = await request(fetchImpl, accountId, readToken, `/workers/workers/${encodeURIComponent(WORKER_NAME)}`, { allowWorkerNotFound: true });
  return response.absent ? null : assertWorker(response.result);
}

export async function verifySmokeEmptyTarget({ approvedAccountId, readToken, expectedZoneIds, expectedWorkerId, fetchImpl = fetch }) {
  if (!/^[a-f0-9]{32}$/i.test(approvedAccountId ?? '') || !approvedAccountId.toLowerCase().endsWith(ACCOUNT_SUFFIX)) fail('approved account configuration is missing or invalid');
  if (typeof readToken !== 'string' || readToken.length < 20) fail('bootstrap read-only credential is missing');
  const zones = parseExpectedZoneIds(expectedZoneIds);
  const worker = await readWorker(fetchImpl, approvedAccountId, readToken);
  if (!worker) fail('Smoke Worker is not initialized; no upload was attempted');
  if (expectedWorkerId !== undefined && worker.id !== expectedWorkerId) fail('Smoke Worker identity differs from the initialization receipt');
  await assertNoRoutes(fetchImpl, approvedAccountId, readToken, zones);
  await assertNoVersionsAndDeployments(fetchImpl, approvedAccountId, readToken, worker.id);
  return { workerId: worker.id, workerName: worker.name };
}

export async function verifySmokePostUpload({ approvedAccountId, readToken, expectedZoneIds, expectedWorkerId, fetchImpl = fetch }) {
  if (!/^[a-f0-9]{32}$/i.test(approvedAccountId ?? '') || !approvedAccountId.toLowerCase().endsWith(ACCOUNT_SUFFIX)) fail('approved account configuration is missing or invalid');
  if (typeof readToken !== 'string' || readToken.length < 20) fail('bootstrap read-only credential is missing');
  const zones = parseExpectedZoneIds(expectedZoneIds);
  const worker = await readWorker(fetchImpl, approvedAccountId, readToken);
  if (!worker) fail('Smoke Worker disappeared after version upload');
  if (expectedWorkerId !== undefined && worker.id !== expectedWorkerId) fail('Smoke Worker identity changed after version upload');
  await assertNoRoutes(fetchImpl, approvedAccountId, readToken, zones);
  await assertNoDeployments(fetchImpl, approvedAccountId, readToken);
  return { workerId: worker.id, workerName: worker.name };
}

export function createInitReceipt({ approvedAccountId, workerId, status, repository, sourceSha, runId, runAttempt, event, branch }) {
  if (!/^[a-f0-9]{32}$/i.test(approvedAccountId ?? '') || !approvedAccountId.toLowerCase().endsWith(ACCOUNT_SUFFIX)) fail('approved account configuration is missing or invalid');
  if (!/^[a-f0-9]{32}$/i.test(workerId ?? '') || !['created', 'already-present'].includes(status)) fail('verified smoke Worker result is invalid');
  if (repository !== 'developing-today/code' || event !== 'workflow_dispatch' || branch !== 'main' ||
      !/^[a-f0-9]{40}$/.test(sourceSha ?? '') || !/^\d{1,20}$/.test(String(runId ?? '')) ||
      !Number.isInteger(runAttempt) || runAttempt < 1) fail('trusted workflow run metadata is invalid');
  return {
    schema: 1,
    repository,
    workflow: INIT_WORKFLOW,
    event,
    branch,
    source_sha: sourceSha,
    run_id: String(runId),
    run_attempt: runAttempt,
    account_id_sha256: createHash('sha256').update(approvedAccountId.toLowerCase()).digest('hex'),
    worker_name: WORKER_NAME,
    worker_id: workerId.toLowerCase(),
    status,
  };
}

export async function bootstrapSmoke({ approvedAccountId, writeToken, readToken, expectedZoneIds, fetchImpl = fetch }) {
  const approved = validateInputs({ approvedAccountId, writeToken, readToken, expectedZoneIds });
  const accountId = approved.approvedAccountId;

  const before = await readWorker(fetchImpl, accountId, readToken);
  if (before) {
    await assertNoRoutes(fetchImpl, accountId, readToken, approved.expectedZoneIds);
    await assertNoVersionsAndDeployments(fetchImpl, accountId, readToken, before.id);
    return { status: 'already-present', workerId: before.id };
  }

  // The complete account route inventory is checked immediately before the
  // only write. Any read/scope/pagination error exits before creation.
  await assertNoRoutes(fetchImpl, accountId, readToken, approved.expectedZoneIds);
  const created = await request(fetchImpl, accountId, writeToken, '/workers/workers', { method: 'POST', body: CREATE_BODY });
  assertWorker(created.result);

  // Re-read using the read-only token, never the create credential.
  const after = await readWorker(fetchImpl, accountId, readToken);
  if (!after || after.id !== created.result.id) fail('Created Worker identity does not match read-only provider readback');
  await assertNoRoutes(fetchImpl, accountId, readToken, approved.expectedZoneIds);
  await assertNoVersionsAndDeployments(fetchImpl, accountId, readToken, after.id);
  return { status: 'created', workerId: after.id };
}

if (process.argv[1] && new URL(import.meta.url).pathname === process.argv[1]) {
  try {
    const approvedAccountId = process.env.CF_SMOKE_INIT_APPROVED_ACCOUNT_ID;
    const result = await bootstrapSmoke({
      approvedAccountId: process.env.CF_SMOKE_INIT_APPROVED_ACCOUNT_ID,
      expectedZoneIds: process.env.CF_SMOKE_INIT_EXPECTED_ZONE_IDS,
      writeToken: process.env.CF_SMOKE_INIT_WRITE_TOKEN,
      readToken: process.env.CF_SMOKE_INIT_READ_TOKEN,
    });
    const receiptPath = process.env.BOOTSTRAP_RECEIPT_PATH;
    if (!receiptPath) fail('init receipt output path is missing');
    const receipt = createInitReceipt({
      approvedAccountId,
      workerId: result.workerId,
      status: result.status,
      repository: process.env.GITHUB_REPOSITORY,
      sourceSha: process.env.GITHUB_SHA,
      runId: process.env.GITHUB_RUN_ID,
      runAttempt: Number(process.env.GITHUB_RUN_ATTEMPT),
      event: process.env.GITHUB_EVENT_NAME,
      branch: process.env.GITHUB_REF_NAME,
    });
    await writeFile(receiptPath, `${JSON.stringify(receipt)}\n`, { flag: 'wx', mode: 0o600 });
    console.log(`Smoke Worker init preflight passed (${result.status}); no code or deployment was uploaded.`);
  } catch (error) {
    console.error(error instanceof Error ? error.message : 'Smoke Worker initialization failed');
    process.exitCode = 1;
  }
}
