import test from 'node:test';
import assert from 'node:assert/strict';
import {
  assertLegacyVersionsEmpty,
  assertNoTargetOverrides,
  assertPrebuiltSmokePayload,
  assertDryRunOutput,
  assertExactVersionReadback,
  assertLegacyVersionIdMatch,
  assertInitCompatibility,
  extractUploadedVersion,
  fetchInitRunEvidence,
  INIT_COMPAT_PATHS,
  readLegacyVersions,
  readVersions,
  validateInitReceipt,
} from './smoke-upload.mjs';
import { createInitReceipt, verifySmokeEmptyTarget, WORKER_NAME, CREATE_BODY } from './bootstrap-smoke.mjs';

const accountId = `${'a'.repeat(28)}cbfa`;
const zoneId = '1'.repeat(32);
const workerId = '0123456789abcdef0123456789abcdef';
const sourceSha = 'a'.repeat(40);
const receiptDigest = 'b'.repeat(64);
const readToken = 'synthetic-read-token-0000000';
const ok = (result, resultInfo) => new Response(JSON.stringify({ success: true, result, ...(resultInfo === undefined ? {} : { result_info: resultInfo }) }));
const worker = () => ({ id: workerId, name: WORKER_NAME, logpush: false, observability: { enabled: false }, subdomain: { enabled: false, previews_enabled: false }, deployed_on: null, references: { domains: [] } });

function uploadFixture({ versions = [], versionPages, route, deployments = [], workerRecord = worker(), omitMetadata = false } = {}) {
  const calls = [];
  const fetchImpl = async (url, init = {}) => {
    const parsed = new URL(url);
    calls.push({ parsed, init });
    if (parsed.pathname === '/client/v4/zones') return ok([{ id: zoneId, account: { id: accountId } }], omitMetadata ? undefined : { total_pages: 1, total_count: 1, page: 1, per_page: 50, count: 1 });
    if (parsed.pathname === `/client/v4/zones/${zoneId}/workers/routes`) return ok(route ? [route] : []);
    if (parsed.pathname.endsWith(`/workers/workers/${WORKER_NAME}`)) return ok(workerRecord);
    if (parsed.pathname.endsWith(`/workers/scripts/${WORKER_NAME}/deployments`)) return ok({ deployments });
    if (parsed.pathname.endsWith(`/workers/workers/${workerId}/versions`)) {
      if (versionPages) {
        const page = Number(parsed.searchParams.get('page'));
        const result = versionPages[page - 1] ?? [];
        return ok(result, omitMetadata ? undefined : { page, per_page: 100, count: result.length, total_count: versionPages.flat().length, total_pages: versionPages.length });
      }
      return ok(versions, omitMetadata ? undefined : { page: 1, per_page: 100, count: versions.length, total_count: versions.length, total_pages: versions.length ? 1 : 0 });
    }
    throw new Error(`unexpected request ${parsed.pathname}`);
  };
  return { calls, fetchImpl };
}

function trustedEvidence() {
  const receipt = createInitReceipt({ approvedAccountId: accountId, workerId, status: 'created', repository: 'developing-today/code', sourceSha, runId: '123456789', runAttempt: 1, event: 'workflow_dispatch', branch: 'main' });
  const run = { id: 123456789, run_attempt: 1, status: 'completed', conclusion: 'success', event: 'workflow_dispatch', head_branch: 'main', head_sha: sourceSha, path: '.github/workflows/private-laptop-bridge-smoke-init.yml@refs/heads/main', repository: { full_name: 'developing-today/code' } };
  const names = ['Run offline bootstrap contract tests', 'Create only the fixed empty Worker and verify it read-only', 'Preserve verified smoke initialization receipt'];
  const jobs = { total_count: 1, jobs: [{ name: 'initialize', run_id: run.id, head_sha: sourceSha, status: 'completed', conclusion: 'success', steps: names.map(name => ({ name, status: 'completed', conclusion: 'success' })) }] };
  const uploadSha = 'c'.repeat(40);
  const compatibility = {
    comparison: { status: 'ahead', ahead_by: 1, behind_by: 0, total_commits: 1, base_commit: { sha: sourceSha }, merge_base_commit: { sha: sourceSha } },
    uploadCommit: { sha: uploadSha },
    files: INIT_COMPAT_PATHS.map(path => ({ path, init_blob_sha: 'd'.repeat(40), upload_blob_sha: 'd'.repeat(40) })),
  };
  return { receiptJson: `${JSON.stringify(receipt)}\n`, run, jobs, compatibility, uploadSha };
}

test('successful init receipt allows main to advance only when the run SHA is an ancestor with identical initializer files', () => {
  const { receiptJson, run, jobs, compatibility, uploadSha } = trustedEvidence();
  const validated = validateInitReceipt({ receiptJson, runJson: JSON.stringify(run), jobsJson: JSON.stringify(jobs), compatibilityJson: JSON.stringify(compatibility), expectedAccountId: accountId, currentSha: uploadSha, initRunId: '123456789' });
  assert.equal(validated.workerId, workerId);
  assert.match(validated.receiptDigest, /^[a-f0-9]{64}$/);
  for (const edit of [
    value => { value.run.status = 'in_progress'; },
    value => { value.run.head_sha = 'c'.repeat(40); },
    value => { value.run.event = 'pull_request'; },
    value => { value.run.path = '.github/workflows/other.yml@refs/heads/main'; },
    value => { value.jobs.jobs[0].steps[1].conclusion = 'failure'; },
  ]) {
    const evidence = trustedEvidence(); edit(evidence);
    assert.throws(() => validateInitReceipt({ receiptJson: evidence.receiptJson, runJson: JSON.stringify(evidence.run), jobsJson: JSON.stringify(evidence.jobs), compatibilityJson: JSON.stringify(evidence.compatibility), expectedAccountId: accountId, currentSha: evidence.uploadSha, initRunId: '123456789' }));
  }
  assert.throws(() => validateInitReceipt({ receiptJson, runJson: JSON.stringify(run), jobsJson: JSON.stringify(jobs), compatibilityJson: JSON.stringify(compatibility), expectedAccountId: `${'f'.repeat(28)}cbfa`, currentSha: uploadSha, initRunId: '123456789' }), /account/);
  const notAncestor = structuredClone(compatibility); notAncestor.comparison.status = 'diverged';
  assert.throws(() => validateInitReceipt({ receiptJson, runJson: JSON.stringify(run), jobsJson: JSON.stringify(jobs), compatibilityJson: JSON.stringify(notAncestor), expectedAccountId: accountId, currentSha: uploadSha, initRunId: '123456789' }), /ancestor/);
  const wrongBase = structuredClone(compatibility); wrongBase.comparison.base_commit.sha = 'e'.repeat(40);
  assert.throws(() => validateInitReceipt({ receiptJson, runJson: JSON.stringify(run), jobsJson: JSON.stringify(jobs), compatibilityJson: JSON.stringify(wrongBase), expectedAccountId: accountId, currentSha: uploadSha, initRunId: '123456789' }), /ancestor/);
  const wrongMergeBase = structuredClone(compatibility); wrongMergeBase.comparison.merge_base_commit.sha = 'e'.repeat(40);
  assert.throws(() => validateInitReceipt({ receiptJson, runJson: JSON.stringify(run), jobsJson: JSON.stringify(jobs), compatibilityJson: JSON.stringify(wrongMergeBase), expectedAccountId: accountId, currentSha: uploadSha, initRunId: '123456789' }), /ancestor/);
  const behind = structuredClone(compatibility); behind.comparison.behind_by = 1;
  assert.throws(() => validateInitReceipt({ receiptJson, runJson: JSON.stringify(run), jobsJson: JSON.stringify(jobs), compatibilityJson: JSON.stringify(behind), expectedAccountId: accountId, currentSha: uploadSha, initRunId: '123456789' }), /ancestor/);
  const wrongCount = structuredClone(compatibility); wrongCount.comparison.total_commits = 2;
  assert.throws(() => validateInitReceipt({ receiptJson, runJson: JSON.stringify(run), jobsJson: JSON.stringify(jobs), compatibilityJson: JSON.stringify(wrongCount), expectedAccountId: accountId, currentSha: uploadSha, initRunId: '123456789' }), /ancestor/);
  const emptyAhead = structuredClone(compatibility); emptyAhead.comparison.ahead_by = 0; emptyAhead.comparison.total_commits = 0;
  assert.throws(() => validateInitReceipt({ receiptJson, runJson: JSON.stringify(run), jobsJson: JSON.stringify(jobs), compatibilityJson: JSON.stringify(emptyAhead), expectedAccountId: accountId, currentSha: uploadSha, initRunId: '123456789' }), /inconsistent/);
  const wrongUploadSha = structuredClone(compatibility); wrongUploadSha.uploadCommit.sha = 'e'.repeat(40);
  assert.throws(() => validateInitReceipt({ receiptJson, runJson: JSON.stringify(run), jobsJson: JSON.stringify(jobs), compatibilityJson: JSON.stringify(wrongUploadSha), expectedAccountId: accountId, currentSha: uploadSha, initRunId: '123456789' }), /ancestor/);
  const identical = structuredClone(compatibility);
  identical.comparison.status = 'identical'; identical.comparison.ahead_by = 0; identical.comparison.total_commits = 0;
  identical.comparison.base_commit.sha = sourceSha; identical.comparison.merge_base_commit.sha = sourceSha; identical.uploadCommit.sha = sourceSha;
  assert.equal(assertInitCompatibility({ compatibility: identical, initSha: sourceSha, currentSha: sourceSha }), true);
  const falseIdentical = structuredClone(identical); falseIdentical.uploadCommit.sha = uploadSha;
  assert.throws(() => assertInitCompatibility({ compatibility: falseIdentical, initSha: sourceSha, currentSha: uploadSha }), /inconsistent/);
  const changedInitializer = structuredClone(compatibility); changedInitializer.files[1].upload_blob_sha = 'e'.repeat(40);
  assert.throws(() => validateInitReceipt({ receiptJson, runJson: JSON.stringify(run), jobsJson: JSON.stringify(jobs), compatibilityJson: JSON.stringify(changedInitializer), expectedAccountId: accountId, currentSha: uploadSha, initRunId: '123456789' }), /initializer compatibility changed/);
  assert.throws(() => assertInitCompatibility({ compatibility: {}, initSha: sourceSha, currentSha: uploadSha }), /ancestor/);
});

test('init evidence fetch is fixed to repository and receipt attempt', async () => {
  const { receiptJson, run, uploadSha } = trustedEvidence();
  const urls = [];
  const evidence = await fetchInitRunEvidence({ receiptJson, initRunId: '123456789', currentSha: uploadSha, githubToken: 'synthetic-github-token-000000', fetchImpl: async (url, options) => {
    urls.push(String(url));
    assert.equal(options.headers.authorization, 'Bearer synthetic-github-token-000000');
    const parsed = new URL(String(url));
    const json = value => new Response(JSON.stringify(value));
    if (parsed.pathname.endsWith('/actions/runs/123456789')) return json(run);
    if (parsed.pathname.endsWith('/attempts/1/jobs')) return json({ jobs: [], total_count: 0 });
    if (parsed.pathname.endsWith(`/compare/${sourceSha}...${uploadSha}`)) return json({ status: 'ahead', ahead_by: 1, behind_by: 0, total_commits: 1, base_commit: { sha: sourceSha }, merge_base_commit: { sha: sourceSha } });
    if (parsed.pathname.endsWith(`/commits/${uploadSha}`)) return json({ sha: uploadSha });
    if (parsed.pathname.includes('/contents/')) return json({ type: 'file', path: decodeURIComponent(parsed.pathname.split('/contents/')[1]), sha: 'd'.repeat(40) });
    throw new Error(`unexpected GitHub API URL ${url}`);
  } });
  assert.equal(urls[0], 'https://api.github.com/repos/developing-today/code/actions/runs/123456789');
  assert.match(urls[1], /attempts\/1\/jobs\?per_page=100$/);
  assert.equal(evidence.jobs.total_count, 0);
  assert.ok(urls.includes(`https://api.github.com/repos/developing-today/code/compare/${sourceSha}...${uploadSha}`));
  assert.ok(urls.includes(`https://api.github.com/repos/developing-today/code/commits/${uploadSha}`));
  assert.equal(evidence.compatibility.comparison.head_commit, undefined);
  assert.equal(evidence.compatibility.uploadCommit.sha, uploadSha);
  assert.equal(evidence.compatibility.files.length, INIT_COMPAT_PATHS.length);
  await assert.rejects(fetchInitRunEvidence({ receiptJson, initRunId: 'not-an-id', currentSha: uploadSha, githubToken: 'synthetic-github-token-000000', fetchImpl: async () => new Response('{}') }), /run ID/);
});

test('pinned cf dry-run must show the fixed disabled smoke config', () => {
  assert.equal(assertDryRunOutput('env.ENABLED ("false")\nenv.GATEWAY_HOST ("private-bridge-smoke.invalid")\nenv.GATEWAY_ACCOUNT_ID ("")\nenv.GATEWAY_POLICY_ID ("")\nenv.START_MS ("")\nenv.END_MS ("")\n--dry-run: exiting now.'), true);
  assert.throws(() => assertDryRunOutput('--dry-run: exiting now. private-laptop-bridge-smoke'), /did not validate/);
  assert.deepEqual(CREATE_BODY.subdomain, { enabled: false, previews_enabled: false });
});

function builtSmokeFixture() {
  const env = {
    ENABLED: { type: 'text', value: 'false' },
    GATEWAY_HOST: { type: 'text', value: 'private-bridge-smoke.invalid' },
    GATEWAY_ACCOUNT_ID: { type: 'text', value: '' },
    GATEWAY_POLICY_ID: { type: 'text', value: '' },
    START_MS: { type: 'text', value: '' },
    END_MS: { type: 'text', value: '' },
  };
  return {
    buildConfig: { producer: { name: 'wrangler', version: '4.166.0' }, buildContext: { isPreview: false } },
    workerNames: ['default'],
    bundleFiles: ['smoke.js', 'smoke.js.map'],
    workerConfig: {
      name: WORKER_NAME, compatibilityDate: '2026-07-30', domains: [], triggers: [], limits: { cpuMs: 10 },
      observability: { enabled: false, logs: { enabled: false }, traces: { enabled: false } }, workersDev: false, previewUrls: false, env,
      manifest: { type: 'complete', mainModule: 'smoke.js', modules: { 'smoke.js': { type: 'esm' }, 'smoke.js.map': { type: 'sourcemap' } } },
    },
  };
}

test('effective Build Output contract excludes routes, domains, triggers, previews, DOs, and extra executable modules', () => {
  const fixture = builtSmokeFixture();
  assert.equal(assertPrebuiltSmokePayload(fixture), true);
  for (const mutate of [
    f => { f.workerConfig.domains = [{ hostname: 'public.invalid' }]; },
    f => { f.workerConfig.triggers = [{ cron: '*/1 * * * *' }]; },
    f => { f.workerConfig.routes = [{ pattern: 'public.invalid/*' }]; },
    f => { f.workerConfig.previewUrls = true; },
    f => { f.workerConfig.durable_objects = { bindings: [{ name: 'BRIDGE', class_name: 'Bridge' }] }; },
    f => { f.workerConfig.env.EXTRA = { type: 'service', service: 'other-worker' }; },
    f => { f.workerConfig.manifest.modules['extra.wasm'] = { type: 'compiled-wasm' }; },
    f => { f.buildConfig.buildContext.isPreview = true; },
    f => { f.workerNames.push('other'); },
  ]) {
    const changed = structuredClone(fixture); mutate(changed);
    assert.throws(() => assertPrebuiltSmokePayload(changed));
  }
});

test('inherited target and mode override variables are rejected', () => {
  assert.equal(assertNoTargetOverrides({}), true);
  for (const name of ['CLOUDFLARE_CI_OVERRIDE_WORKER_NAME','CF_WORKER_NAME','CLOUDFLARE_WORKER_NAME','WRANGLER_WORKER_NAME','CF_MODE','CLOUDFLARE_ENV','WRANGLER_ENV']) {
    assert.throws(() => assertNoTargetOverrides({ [name]: 'wrong-target' }), new RegExp(name));
  }
});

test('upload response must be the pinned exact version-upload schema and UUID', () => {
  const good = extractUploadedVersion(JSON.stringify({ type: 'version-upload', version: 1, worker_name: WORKER_NAME, worker_name_overridden: false, wrangler_environment: null, preview_url: null, preview_alias_url: null, version_id: '00000000-0000-4000-8000-000000000001' }), { receiptDigest, sourceSha });
  assert.equal(good.versionId, '00000000-0000-4000-8000-000000000001');
  assert.equal(good.message, `init_receipt_sha256=${receiptDigest}`);
  for (const response of [
    { type: 'version-upload', version: 1, worker_name: 'wrong', worker_name_overridden: false, version_id: good.versionId },
    { type: 'version-upload', version: 1, worker_name: WORKER_NAME, worker_name_overridden: false, version_id: 'latest' },
    { type: 'version-upload', version: 2, worker_name: WORKER_NAME, worker_name_overridden: false, version_id: good.versionId },
    { type: 'version-upload', version: 1, worker_name: WORKER_NAME, worker_name_overridden: true, version_id: good.versionId },
    { type: 'version-upload', version: 1, worker_name: WORKER_NAME, worker_name_overridden: false, preview_url: 'https://preview.invalid', version_id: good.versionId },
    { type: 'version-upload', version: 1, worker_name: WORKER_NAME, worker_name_overridden: false, preview_alias_url: 'https://alias.invalid', version_id: good.versionId },
    { type: 'version-upload', version: 1, worker_name: WORKER_NAME, worker_name_overridden: false, wrangler_environment: 'production', version_id: good.versionId },
  ]) assert.throws(() => extractUploadedVersion(JSON.stringify(response), { receiptDigest, sourceSha }));
});

test('legacy scripts version list is empty before upload and pinned to the exact Worker name', async () => {
  const seen = [];
  const empty = await assertLegacyVersionsEmpty({ accountId, readToken, fetchImpl: async url => {
    seen.push(String(url)); return ok({ items: [] }, { total_count: 0, count: 0, page: 1, per_page: 100, total_pages: 0 });
  } });
  assert.equal(empty, true);
  assert.equal(seen[0], `https://api.cloudflare.com/client/v4/accounts/${accountId}/workers/scripts/${WORKER_NAME}/versions?page=1&per_page=100`);
  await assert.rejects(assertLegacyVersionsEmpty({ accountId, readToken, fetchImpl: async () => ok({ items: [{ id: '00000000-0000-4000-8000-000000000099', metadata: {}, number: 1 }] }) }), /not empty/);
  await assert.rejects(assertLegacyVersionsEmpty({ accountId, readToken, fetchImpl: async () => ok({ items: [] }, { total_count: 1 }) }), /ended before reported/);
  await assert.rejects(assertLegacyVersionsEmpty({ accountId, readToken, fetchImpl: async () => ok({ items: [] }, { page: 2 }) }), /page does not match/);
  await assert.rejects(assertLegacyVersionsEmpty({ accountId, readToken, fetchImpl: async () => ok({}) }), /ambiguous response shape/);
  await assert.rejects(assertLegacyVersionsEmpty({ accountId, readToken, fetchImpl: async () => new Response('{}', { status: 403 }) }), /authorization denied/);
});

test('legacy and Beta version lists agree by exact UUID while Beta carries provenance', async () => {
  const versionId = '00000000-0000-4000-8000-000000000001';
  const legacyItem = { id: versionId, resources: {}, metadata: { created_on: '2026-11-23T00:00:00Z' }, number: 1 };
  const betaItem = { id: versionId, annotations: { 'workers/tag': sourceSha, 'workers/message': `init_receipt_sha256=${receiptDigest}` }, urls: [], hasPreview: false };
  const paths = [];
  const fetchImpl = async url => {
    const parsed = new URL(url); paths.push(parsed.pathname);
    if (parsed.pathname.endsWith(`/workers/scripts/${WORKER_NAME}/versions`)) return ok({ items: [legacyItem] }, { page: 1, per_page: 100, count: 1, total_count: 1, total_pages: 1 });
    if (parsed.pathname.endsWith(`/workers/workers/${workerId}/versions`)) return ok([betaItem], { page: 1, per_page: 100, count: 1, total_count: 1, total_pages: 1 });
    throw new Error(`unexpected URL ${parsed.pathname}`);
  };
  const legacyVersions = await readLegacyVersions({ accountId, readToken, fetchImpl });
  const betaVersions = await readVersions({ accountId, workerId, readToken, fetchImpl });
  assert.equal(legacyVersions[0].annotations, undefined); // Legacy list deliberately has no annotation fields.
  assertLegacyVersionIdMatch({ legacyVersions, betaVersions, versionId });
  assertExactVersionReadback({ versions: betaVersions, versionId, sourceSha, receiptDigest });
  assert.ok(paths.some(path => path.endsWith(`/workers/scripts/${WORKER_NAME}/versions`)));
  assert.ok(paths.some(path => path.endsWith(`/workers/workers/${workerId}/versions`)));
  assert.throws(() => assertLegacyVersionIdMatch({ legacyVersions: [{ ...legacyItem, id: '00000000-0000-4000-8000-000000000002' }], betaVersions, versionId }), /do not agree/);
  assert.throws(() => assertLegacyVersionIdMatch({ legacyVersions: [legacyItem, { ...legacyItem, id: '00000000-0000-4000-8000-000000000003' }], betaVersions, versionId }), /do not agree/);
  assert.throws(() => assertExactVersionReadback({ versions: [{ ...betaItem, annotations: {} }], versionId, sourceSha, receiptDigest }), /does not bind/);
});

test('version readback tolerates omitted optional pagination metadata but proves complete list', async () => {
  const f = uploadFixture({ versions: [{ id: 'old' }], omitMetadata: true });
  const result = await readVersions({ accountId, workerId, readToken, fetchImpl: f.fetchImpl });
  assert.deepEqual(result, [{ id: 'old' }]);
  assert.equal(f.calls.length, 1);

  const pages = uploadFixture({ versionPages: [Array.from({ length: 100 }, (_, index) => ({ id: `v${index}` })), [{ id: 'last' }]] });
  assert.equal((await readVersions({ accountId, workerId, readToken, fetchImpl: pages.fetchImpl })).length, 101);
  assert.equal(pages.calls.length, 2);
});

test('version pagination mismatch and unavailable readback fail closed', async () => {
  const mismatch = uploadFixture();
  mismatch.fetchImpl = async url => new Response(JSON.stringify({ success: true, result: [], result_info: { page: 2, total_count: 0 } }));
  await assert.rejects(readVersions({ accountId, workerId, readToken, fetchImpl: mismatch.fetchImpl }), /page does not match/);
  await assert.rejects(readVersions({ accountId, workerId, readToken, fetchImpl: async () => new Response('{}', { status: 403 }) }), /authorization denied/);
  await assert.rejects(readVersions({ accountId, workerId, readToken, fetchImpl: async () => { throw new Error('offline'); } }), /request failed/);
});

test('preflight checks exact unrouted target and rejects identity, route, or existing version state', async () => {
  const good = uploadFixture();
  const result = await readVersions({ accountId, workerId, readToken, fetchImpl: good.fetchImpl });
  assert.deepEqual(result, []);

  const wrongWorker = uploadFixture({ workerRecord: { ...worker(), id: 'f'.repeat(32) } });
  await assert.rejects(verifySmokeEmptyTarget({ approvedAccountId: accountId, expectedZoneIds: [zoneId], readToken, expectedWorkerId: workerId, fetchImpl: wrongWorker.fetchImpl }), /identity differs/);
  const routed = uploadFixture({ route: { id: 'route', pattern: 'example.invalid/*', script: WORKER_NAME } });
  await assert.rejects(verifySmokeEmptyTarget({ approvedAccountId: accountId, expectedZoneIds: [zoneId], readToken, expectedWorkerId: workerId, fetchImpl: routed.fetchImpl }), /route already targets/);
  const nonempty = uploadFixture({ versions: [{ id: 'old' }] });
  await assert.rejects(verifySmokeEmptyTarget({ approvedAccountId: accountId, expectedZoneIds: [zoneId], readToken, expectedWorkerId: workerId, fetchImpl: nonempty.fetchImpl }), /already has a version/);
});

test('post-upload proof requires exactly the returned UUID, provenance, no public URLs, and no preview', () => {
  const versionId = '00000000-0000-4000-8000-000000000001';
  const valid = { id: versionId, annotations: { 'workers/tag': sourceSha, 'workers/message': `init_receipt_sha256=${receiptDigest}` }, urls: [], hasPreview: false };
  assert.equal(assertExactVersionReadback({ versionId, versions: [valid], sourceSha, receiptDigest }), true);
  assert.throws(() => assertExactVersionReadback({ versionId, versions: [valid, { id: 'other' }], sourceSha, receiptDigest }), /version set is not exactly one/);
  assert.throws(() => assertExactVersionReadback({ versionId, versions: [{ ...valid, urls: ['https://preview.invalid'] }], sourceSha, receiptDigest }), /routable or preview URL/);
  assert.throws(() => assertExactVersionReadback({ versionId, versions: [{ ...valid, preview_url: 'https://preview.invalid' }], sourceSha, receiptDigest }), /routable or preview URL/);
  assert.throws(() => assertExactVersionReadback({ versionId, versions: [{ ...valid, hasPreview: true }], sourceSha, receiptDigest }), /routable or preview URL/);
  assert.throws(() => assertExactVersionReadback({ versionId, versions: [{ ...valid, annotations: {} }], sourceSha, receiptDigest }), /does not bind/);
});
