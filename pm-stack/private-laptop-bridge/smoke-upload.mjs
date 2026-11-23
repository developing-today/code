#!/usr/bin/env node
// Upload one immutable smoke Worker version and verify it remains unrouted.
import { createHash } from 'node:crypto';
import { appendFile, readFile, readdir, writeFile } from 'node:fs/promises';
import { API, INIT_WORKFLOW, WORKER_NAME, verifySmokeEmptyTarget, verifySmokePostUpload } from './bootstrap-smoke.mjs';

const REPOSITORY = 'developing-today/code';
export const UPLOAD_WORKFLOW = '.github/workflows/private-laptop-bridge-smoke-upload.yml';
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const SHA = /^[0-9a-f]{40}$/;
const HASH = /^[0-9a-f]{64}$/;
const TARGET_OVERRIDE_ENV = [
  'CLOUDFLARE_CI_OVERRIDE_WORKER_NAME',
  'CF_WORKER_NAME',
  'CLOUDFLARE_WORKER_NAME',
  'WRANGLER_WORKER_NAME',
  'CF_MODE',
  'CLOUDFLARE_ENV',
  'WRANGLER_ENV',
];
const fail = message => { throw new Error(message); };
const hash = value => createHash('sha256').update(value).digest('hex');
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

function parseJson(value, label) {
  try { return typeof value === 'string' ? JSON.parse(value) : value; }
  catch { fail(`${label} is not valid JSON`); }
}

function hasNonemptyPreviewUrl(value) {
  return value && typeof value === 'object' && Object.entries(value).some(([key, field]) =>
    key.toLowerCase().includes('preview') && key.toLowerCase().includes('url') &&
    field !== undefined && field !== null && field !== '' && (!Array.isArray(field) || field.length !== 0));
}

function receiptShape(receipt) {
  const expectedKeys = ['account_id_sha256','branch','event','repository','run_attempt','run_id','schema','source_sha','status','worker_id','worker_name','workflow'];
  if (!receipt || typeof receipt !== 'object' || Array.isArray(receipt) || !same(Object.keys(receipt).sort(), expectedKeys)) fail('init receipt schema is missing or ambiguous');
  if (receipt.schema !== 1 || receipt.repository !== REPOSITORY || receipt.workflow !== INIT_WORKFLOW || receipt.event !== 'workflow_dispatch' ||
      receipt.branch !== 'main' || !SHA.test(receipt.source_sha ?? '') || !/^\d{1,20}$/.test(String(receipt.run_id)) ||
      !Number.isInteger(receipt.run_attempt) || receipt.run_attempt < 1 || !HASH.test(receipt.account_id_sha256 ?? '') ||
      receipt.worker_name !== WORKER_NAME || !/^[a-f0-9]{32}$/i.test(receipt.worker_id ?? '') ||
      !['created','already-present'].includes(receipt.status)) fail('init receipt fields do not identify the fixed trusted target');
  return receipt;
}

export function validateInitReceipt({ receiptJson, runJson, jobsJson, expectedAccountId, currentSha, initRunId }) {
  const receipt = receiptShape(parseJson(receiptJson, 'init receipt'));
  const run = parseJson(runJson, 'init workflow run');
  const jobsResponse = parseJson(jobsJson, 'init workflow jobs');
  if (!/^[a-f0-9]{32}$/i.test(expectedAccountId ?? '') || !expectedAccountId.toLowerCase().endsWith('cbfa') ||
      receipt.account_id_sha256 !== hash(expectedAccountId.toLowerCase())) fail('init receipt account does not match protected upload configuration');
  if (!SHA.test(currentSha ?? '') || receipt.source_sha !== currentSha || receipt.run_id !== String(initRunId)) fail('init receipt does not match this exact main dispatch SHA and run ID');
  if (!run || !Number.isSafeInteger(run.id) || String(run.id) !== String(initRunId) || run.run_attempt !== receipt.run_attempt || run.status !== 'completed' || run.conclusion !== 'success' ||
      run.event !== 'workflow_dispatch' || run.head_branch !== 'main' || run.head_sha !== currentSha || run.path?.split('@')[0] !== INIT_WORKFLOW ||
      run.repository?.full_name !== REPOSITORY) fail('init receipt is not from the exact successful protected-main workflow run');
  const jobs = jobsResponse?.jobs;
  if (!Array.isArray(jobs) || jobsResponse.total_count !== 1 || jobs.length !== 1) fail('init workflow job evidence is missing or ambiguous');
  const job = jobs[0];
  if (job.name !== 'initialize' || job.run_id !== run.id || job.head_sha !== currentSha || job.status !== 'completed' || job.conclusion !== 'success' || !Array.isArray(job.steps)) {
    fail('init workflow job is not the exact successful main job');
  }
  const steps = new Map(job.steps.map(step => [step.name, step]));
  for (const name of ['Run offline bootstrap contract tests','Create only the fixed empty Worker and verify it read-only','Preserve verified smoke initialization receipt']) {
    const step = steps.get(name);
    if (step?.status !== 'completed' || step?.conclusion !== 'success') fail(`init workflow did not complete required evidence step: ${name}`);
  }
  return {
    workerId: receipt.worker_id.toLowerCase(),
    receiptDigest: hash(typeof receiptJson === 'string' ? receiptJson : JSON.stringify(receipt)),
    status: receipt.status,
  };
}

export async function fetchInitRunEvidence({ receiptJson, initRunId, githubToken, fetchImpl = fetch }) {
  const receipt = receiptShape(parseJson(receiptJson, 'init receipt'));
  if (!/^\d{1,20}$/.test(String(initRunId)) || receipt.run_id !== String(initRunId)) fail('init run ID does not match the receipt');
  if (typeof githubToken !== 'string' || githubToken.length < 20) fail('GitHub read-only token is missing');
  const base = `https://api.github.com/repos/${REPOSITORY}/actions/runs/${encodeURIComponent(initRunId)}`;
  async function get(path) {
    let response;
    try {
      response = await fetchImpl(path, { headers: { authorization: `Bearer ${githubToken}`, accept: 'application/vnd.github+json', 'x-github-api-version': '2022-11-28' }, redirect: 'error', signal: AbortSignal.timeout(15_000) });
    } catch { fail('GitHub init-run evidence request failed'); }
    if (!response.ok) fail(`GitHub init-run evidence request denied or unavailable (HTTP ${response.status})`);
    try { return await response.json(); } catch { fail('GitHub init-run evidence was invalid JSON'); }
  }
  const run = await get(base);
  const jobs = await get(`${base}/attempts/${receipt.run_attempt}/jobs?per_page=100`);
  return { run, jobs };
}

export function assertDryRunOutput(text) {
  const requiredBindings = [
    'env.ENABLED ("false")',
    'env.GATEWAY_HOST ("private-bridge-smoke.invalid")',
    'env.GATEWAY_ACCOUNT_ID ("")',
    'env.GATEWAY_POLICY_ID ("")',
    'env.START_MS ("")',
    'env.END_MS ("")',
  ];
  if (typeof text !== 'string' || !text.includes('--dry-run: exiting now.') || requiredBindings.some(binding => !text.includes(binding))) {
    fail('pinned cf prebuilt dry-run did not validate the fixed disabled smoke Worker');
  }
  return true;
}

export function assertNoTargetOverrides(environment = process.env) {
  const set = TARGET_OVERRIDE_ENV.filter(name => typeof environment[name] === 'string' && environment[name] !== '');
  if (set.length) fail(`effective Worker target override variables must be empty: ${set.join(', ')}`);
  return true;
}

export function assertPrebuiltSmokePayload({ buildConfig, workerConfig, workerNames, bundleFiles }) {
  const exactKeys = (value, expected) => value && typeof value === 'object' && !Array.isArray(value) && same(Object.keys(value).sort(), [...expected].sort());
  if (!exactKeys(buildConfig, ['producer', 'buildContext']) || !exactKeys(buildConfig.producer, ['name', 'version']) ||
      buildConfig.producer.name !== 'wrangler' || buildConfig.producer.version !== '4.166.0' ||
      !exactKeys(buildConfig.buildContext, ['isPreview']) || buildConfig.buildContext.isPreview !== false) {
    fail('prebuilt Build Output producer or context is not the pinned non-preview build');
  }
  if (!Array.isArray(workerNames) || workerNames.length !== 1 || workerNames[0] !== 'default') fail('prebuilt Build Output must contain only its default Worker');
  const expectedWorkerKeys = ['name','compatibilityDate','domains','triggers','limits','observability','workersDev','previewUrls','env','manifest'];
  if (!exactKeys(workerConfig, expectedWorkerKeys) || workerConfig.name !== WORKER_NAME || workerConfig.compatibilityDate !== '2026-07-30' ||
      !Array.isArray(workerConfig.domains) || workerConfig.domains.length !== 0 || !Array.isArray(workerConfig.triggers) || workerConfig.triggers.length !== 0 ||
      workerConfig.workersDev !== false || workerConfig.previewUrls !== false || !exactKeys(workerConfig.limits, ['cpuMs']) || workerConfig.limits.cpuMs !== 10 ||
      !exactKeys(workerConfig.observability, ['enabled','logs','traces']) || workerConfig.observability.enabled !== false ||
      !exactKeys(workerConfig.observability.logs, ['enabled']) || workerConfig.observability.logs.enabled !== false ||
      !exactKeys(workerConfig.observability.traces, ['enabled']) || workerConfig.observability.traces.enabled !== false) {
    fail('prebuilt Worker config has a changed target or an unexpected route, domain, trigger, preview, telemetry, DO, or binding field');
  }
  const env = {
    ENABLED: { type: 'text', value: 'false' },
    GATEWAY_HOST: { type: 'text', value: 'private-bridge-smoke.invalid' },
    GATEWAY_ACCOUNT_ID: { type: 'text', value: '' },
    GATEWAY_POLICY_ID: { type: 'text', value: '' },
    START_MS: { type: 'text', value: '' },
    END_MS: { type: 'text', value: '' },
  };
  if (!same(workerConfig.env, env)) fail('prebuilt Worker bindings differ from the fixed disabled smoke fixture');
  const manifest = workerConfig.manifest;
  if (!exactKeys(manifest, ['type','mainModule','modules']) || manifest.type !== 'complete' || manifest.mainModule !== 'smoke.js' ||
      !exactKeys(manifest.modules, ['smoke.js','smoke.js.map']) || manifest.modules['smoke.js']?.type !== 'esm' || manifest.modules['smoke.js.map']?.type !== 'sourcemap') {
    fail('prebuilt Worker manifest is not the fixed stateless smoke entrypoint');
  }
  if (!Array.isArray(bundleFiles) || !same([...bundleFiles].sort(), ['smoke.js','smoke.js.map'])) fail('prebuilt Worker bundle contains an unexpected module or executable payload');
  return true;
}

export async function validatePrebuiltSmokeOutput(outputPath) {
  let buildConfig;
  let workerConfig;
  let workerNames;
  let bundleFiles;
  try {
    buildConfig = JSON.parse(await readFile(`${outputPath}/config.json`, 'utf8'));
    workerConfig = JSON.parse(await readFile(`${outputPath}/workers/default/worker.config.json`, 'utf8'));
    workerNames = await readdir(`${outputPath}/workers`);
    bundleFiles = await readdir(`${outputPath}/workers/default/bundle`);
  } catch { fail('pinned cf Build Output files are missing or invalid'); }
  return assertPrebuiltSmokePayload({ buildConfig, workerConfig, workerNames, bundleFiles });
}

export function extractUploadedVersion(stdout, { receiptDigest, sourceSha }) {
  const body = parseJson(stdout, 'cf version-upload response');
  if (body?.type !== 'version-upload' || body.version !== 1 || body.worker_name !== WORKER_NAME || body.worker_name_overridden !== false ||
      (body.wrangler_environment !== undefined && body.wrangler_environment !== null && body.wrangler_environment !== '') ||
      hasNonemptyPreviewUrl(body) || !UUID.test(body.version_id ?? '')) {
    fail('cf version-upload response does not match the pinned exact Worker schema');
  }
  if (!SHA.test(sourceSha ?? '') || !HASH.test(receiptDigest ?? '')) fail('upload provenance inputs are invalid');
  return { versionId: body.version_id, message: `init_receipt_sha256=${receiptDigest}` };
}

function apiEnvelope(response, body) {
  if (!body || typeof body !== 'object' || Array.isArray(body) || !response.ok || body.success !== true || !Object.hasOwn(body, 'result')) {
    const status = response.status;
    fail(status === 401 || status === 403 ? `Cloudflare verification authorization denied (HTTP ${status})` : `Cloudflare verification request failed (HTTP ${status})`);
  }
  return { result: body.result, resultInfo: body.result_info };
}

export async function readVersions({ accountId, workerId, readToken, fetchImpl = fetch }) {
  const rows = [];
  let totalCount;
  let totalPages;
  for (let page = 1; page <= 100; page += 1) {
    const url = `${API}/accounts/${encodeURIComponent(accountId)}/workers/workers/${encodeURIComponent(workerId)}/versions?page=${page}&per_page=100`;
    let response;
    try { response = await fetchImpl(url, { headers: { authorization: `Bearer ${readToken}`, accept: 'application/json' }, redirect: 'error', signal: AbortSignal.timeout(15_000) }); }
    catch { fail('Cloudflare version verification request failed; upload result is uncertain'); }
    let body; try { body = await response.json(); } catch { fail(`Cloudflare version response is invalid JSON (HTTP ${response.status})`); }
    const { result, resultInfo } = apiEnvelope(response, body);
    if (!Array.isArray(result)) fail('Cloudflare Beta versions response has an ambiguous result schema');
    rows.push(...result);
    if (resultInfo !== undefined) {
      if (!resultInfo || typeof resultInfo !== 'object' || Array.isArray(resultInfo)) fail('Cloudflare version pagination metadata is invalid');
      if (resultInfo.page !== undefined && resultInfo.page !== page) fail('Cloudflare version page does not match request');
      if (resultInfo.per_page !== undefined && resultInfo.per_page !== 100) fail('Cloudflare version page size does not match request');
      if (resultInfo.count !== undefined && resultInfo.count !== result.length) fail('Cloudflare version page count is inconsistent');
      if (resultInfo.total_count !== undefined) {
        if (!Number.isInteger(resultInfo.total_count) || resultInfo.total_count < 0) fail('Cloudflare version total count is invalid');
        if (totalCount === undefined) totalCount = resultInfo.total_count;
        if (totalCount !== resultInfo.total_count) fail('Cloudflare version list changed during pagination');
      }
      if (resultInfo.total_pages !== undefined) {
        if (!Number.isInteger(resultInfo.total_pages) || resultInfo.total_pages < 0 || resultInfo.total_pages > 100) fail('Cloudflare version total pages is invalid');
        if (totalPages === undefined) totalPages = resultInfo.total_pages;
        if (totalPages !== resultInfo.total_pages) fail('Cloudflare version list changed during pagination');
      }
    }
    if (totalCount !== undefined && rows.length > totalCount) fail('Cloudflare version list exceeded reported total count');
    const doneByPages = totalPages !== undefined && (totalPages === 0 ? page === 1 && rows.length === 0 : page >= totalPages);
    const doneByCount = totalCount !== undefined && rows.length === totalCount;
    if (doneByPages || doneByCount || (totalPages === undefined && totalCount === undefined && result.length < 100)) {
      if (totalCount !== undefined && totalCount !== rows.length) fail('Cloudflare version list is incomplete');
      return rows;
    }
    if (result.length < 100 && !doneByCount) fail('Cloudflare version list ended before its reported total count');
  }
  fail('Cloudflare version pagination exceeded hard bound');
}

async function cloudflareRead({ accountId, path, readToken, fetchImpl, label }) {
  const url = `${API}/accounts/${encodeURIComponent(accountId)}${path}`;
  let response;
  try { response = await fetchImpl(url, { headers: { authorization: `Bearer ${readToken}`, accept: 'application/json' }, redirect: 'error', signal: AbortSignal.timeout(15_000) }); }
  catch { fail(`Cloudflare ${label} verification request failed`); }
  let body;
  try { body = await response.json(); } catch { fail(`Cloudflare ${label} response is invalid JSON (HTTP ${response.status})`); }
  return apiEnvelope(response, body);
}

export async function assertLegacyVersionsEmpty({ accountId, readToken, fetchImpl = fetch }) {
  const { result, resultInfo } = await cloudflareRead({ accountId, readToken, fetchImpl, label: 'legacy version list', path: `/workers/scripts/${encodeURIComponent(WORKER_NAME)}/versions?per_page=100&page=1` });
  if (!result || typeof result !== 'object' || Array.isArray(result) || !Array.isArray(result.items) || result.items.length !== 0 ||
      (result.total_count !== undefined && result.total_count !== 0) || (result.count !== undefined && result.count !== 0) ||
      (resultInfo !== undefined && (!resultInfo || typeof resultInfo !== 'object' || Array.isArray(resultInfo) ||
        (resultInfo.total_count !== undefined && resultInfo.total_count !== 0) || (resultInfo.count !== undefined && resultInfo.count !== 0) ||
        (resultInfo.total_pages !== undefined && ![0, 1].includes(resultInfo.total_pages)) ||
        (resultInfo.page !== undefined && resultInfo.page !== 1) || (resultInfo.per_page !== undefined && resultInfo.per_page !== 100)))) {
    fail('legacy Scripts version list is not explicitly empty for the fixed smoke Worker');
  }
  return true;
}

export async function readExactLegacyVersion({ accountId, versionId, readToken, fetchImpl = fetch }) {
  return (await cloudflareRead({ accountId, readToken, fetchImpl, label: 'legacy exact version', path: `/workers/scripts/${encodeURIComponent(WORKER_NAME)}/versions/${encodeURIComponent(versionId)}` })).result;
}

export async function readExactBetaVersion({ accountId, workerId, versionId, readToken, fetchImpl = fetch }) {
  return (await cloudflareRead({ accountId, readToken, fetchImpl, label: 'Beta exact version', path: `/workers/workers/${encodeURIComponent(workerId)}/versions/${encodeURIComponent(versionId)}` })).result;
}

export function assertMatchingLegacyBetaVersions({ legacyVersion, betaVersion, versionId, sourceSha, receiptDigest }) {
  const expectedMessage = `init_receipt_sha256=${receiptDigest}`;
  for (const [label, version] of [['legacy', legacyVersion], ['Beta', betaVersion]]) {
    if (!version || typeof version !== 'object' || Array.isArray(version) || version.id !== versionId ||
        version.annotations?.['workers/tag'] !== sourceSha || version.annotations?.['workers/message'] !== expectedMessage) {
      fail(`${label} exact-version readback does not match the server UUID and provenance`);
    }
  }
  return true;
}

export function assertExactVersionReadback({ versionId, versions, sourceSha, receiptDigest }) {
  if (!UUID.test(versionId ?? '') || !SHA.test(sourceSha ?? '') || !HASH.test(receiptDigest ?? '') || !Array.isArray(versions) || versions.length !== 1) fail('version readback inputs are invalid or the Worker version set is not exactly one');
  const exact = versions.filter(version => version?.id === versionId);
  if (exact.length !== 1) fail('server-returned version UUID is absent or ambiguous on the exact Worker');
  const version = exact[0];
  if (version.annotations?.['workers/tag'] !== sourceSha || version.annotations?.['workers/message'] !== `init_receipt_sha256=${receiptDigest}`) {
    fail('uploaded Worker version metadata does not bind the exact SHA and init receipt');
  }
  if (!Array.isArray(version.urls) || version.urls.length !== 0 || version.hasPreview === true || hasNonemptyPreviewUrl(version)) fail('uploaded Worker version has an unexpected routable or preview URL');
  return true;
}

function required(name) {
  const value = process.env[name];
  if (!value) fail(`required ${name} is missing`);
  return value;
}

async function readJsonFile(path, name) {
  try { return await readFile(path, 'utf8'); } catch { fail(`${name} file is unavailable`); }
}

async function main() {
  const [action, ...args] = process.argv.slice(2);
  if (action === 'validate-init') {
    if (args.length !== 3) fail('usage: validate-init <receipt.json> <run.json> <jobs.json>');
    const result = validateInitReceipt({
      receiptJson: await readJsonFile(args[0], 'init receipt'), runJson: await readJsonFile(args[1], 'init run'), jobsJson: await readJsonFile(args[2], 'init jobs'),
      expectedAccountId: required('CF_SMOKE_UPLOAD_ACCOUNT_ID'), currentSha: required('GITHUB_SHA'), initRunId: required('INIT_RUN_ID'),
    });
    const uploadToken = process.env.CF_SMOKE_UPLOAD_API_TOKEN;
    const readToken = process.env.CF_SMOKE_UPLOAD_READ_TOKEN;
    if (typeof uploadToken !== 'string' || uploadToken.length < 20 || typeof readToken !== 'string' || readToken.length < 20) fail('smoke upload and read-only credentials are both required');
    if (uploadToken === readToken) fail('smoke upload and read-only credentials must be distinct');
    await appendFile(required('GITHUB_OUTPUT'), `worker_id=${result.workerId}\nreceipt_digest=${result.receiptDigest}\n`);
    return;
  }
  if (action === 'fetch-init') {
    if (args.length !== 3) fail('usage: fetch-init <receipt.json> <run.json> <jobs.json>');
    const receiptJson = await readJsonFile(args[0], 'init receipt');
    const evidence = await fetchInitRunEvidence({ receiptJson, initRunId: required('INIT_RUN_ID'), githubToken: required('GITHUB_TOKEN') });
    await writeFile(args[1], `${JSON.stringify(evidence.run)}\n`, { flag: 'wx', mode: 0o600 });
    await writeFile(args[2], `${JSON.stringify(evidence.jobs)}\n`, { flag: 'wx', mode: 0o600 });
    return;
  }
  if (action === 'preflight') {
    if (args.length !== 1) fail('usage: preflight <receipt.json>');
    const receipt = receiptShape(parseJson(await readJsonFile(args[0], 'init receipt'), 'init receipt'));
    const accountId = required('CF_SMOKE_UPLOAD_ACCOUNT_ID');
    if (receipt.account_id_sha256 !== hash(accountId.toLowerCase()) || receipt.source_sha !== required('GITHUB_SHA')) fail('init receipt does not match current upload account or SHA');
    await verifySmokeEmptyTarget({ approvedAccountId: accountId, expectedZoneIds: required('CF_SMOKE_UPLOAD_EXPECTED_ZONE_IDS'), readToken: required('CF_SMOKE_UPLOAD_READ_TOKEN'), expectedWorkerId: receipt.worker_id });
    await assertLegacyVersionsEmpty({ accountId, readToken: required('CF_SMOKE_UPLOAD_READ_TOKEN') });
    console.log('Smoke target identity, empty version/deployment state, routes and preview settings verified read-only.');
    return;
  }
  if (action === 'verify-upload') {
    if (args.length !== 1) fail('usage: verify-upload <cf-response.json>');
    const responseText = await readJsonFile(args[0], 'cf upload response');
    const upload = extractUploadedVersion(responseText, { receiptDigest: required('INIT_RECEIPT_DIGEST'), sourceSha: required('GITHUB_SHA') });
    const accountId = required('CF_SMOKE_UPLOAD_ACCOUNT_ID');
    const workerId = required('INIT_WORKER_ID');
    const readToken = required('CF_SMOKE_UPLOAD_READ_TOKEN');
    const versions = await readVersions({ accountId, workerId, readToken });
    assertExactVersionReadback({ versionId: upload.versionId, versions, sourceSha: required('GITHUB_SHA'), receiptDigest: required('INIT_RECEIPT_DIGEST') });
    const [legacyVersion, betaVersion] = await Promise.all([
      readExactLegacyVersion({ accountId, versionId: upload.versionId, readToken }),
      readExactBetaVersion({ accountId, workerId, versionId: upload.versionId, readToken }),
    ]);
    assertMatchingLegacyBetaVersions({ legacyVersion, betaVersion, versionId: upload.versionId, sourceSha: required('GITHUB_SHA'), receiptDigest: required('INIT_RECEIPT_DIGEST') });
    await verifySmokePostUpload({ approvedAccountId: accountId, expectedZoneIds: required('CF_SMOKE_UPLOAD_EXPECTED_ZONE_IDS'), readToken, expectedWorkerId: workerId });
    console.log(`Verified immutable smoke version ${upload.versionId} in legacy and Beta APIs; no deployment, route or preview was created.`);
    return;
  }
  if (action === 'dry-run') {
    if (args.length !== 1) fail('usage: dry-run <cf-output.txt>');
    assertDryRunOutput(await readJsonFile(args[0], 'cf dry-run output'));
    return;
  }
  if (action === 'guard-target-env') {
    if (args.length !== 0) fail('usage: guard-target-env');
    assertNoTargetOverrides();
    return;
  }
  if (action === 'validate-payload') {
    if (args.length !== 1) fail('usage: validate-payload <build-output-v1-dir>');
    await validatePrebuiltSmokeOutput(args[0]);
    return;
  }
  fail('action must be fetch-init, validate-init, preflight, verify-upload, dry-run, guard-target-env, or validate-payload');
}

if (process.argv[1] && new URL(import.meta.url).pathname === process.argv[1]) {
  main().catch(error => {
    console.error(error instanceof Error ? error.message : 'Smoke version upload validation failed');
    process.exitCode = 1;
  });
}
