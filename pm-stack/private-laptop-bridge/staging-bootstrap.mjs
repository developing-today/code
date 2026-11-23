#!/usr/bin/env node
// Read-only gates for the dormant fixed-target bridge bootstrap workflow.
// This program never writes to Cloudflare. Only the separate reviewed workflow
// may invoke pinned `cf deploy` after these checks pass.
import {createHash} from 'node:crypto';
import {readFile, writeFile} from 'node:fs/promises';
import {assertEffectivePayload} from './release-contract.mjs';

export const CLOUDFLARE_API = 'https://api.cloudflare.com/client/v4';
export const GITHUB_REPOSITORY = 'developing-today/code';
export const RELEASE_ENVIRONMENT = 'cloudflare-staging';
export const BOOTSTRAP_ENVIRONMENT = RELEASE_ENVIRONMENT;
export const ENVIRONMENT_NAME = BOOTSTRAP_ENVIRONMENT;
export const WORKER_NAME = 'private-laptop-bridge-staging';
export const DO_CLASS = 'PrivateLaptopBridge';
export const BOOTSTRAP_WORKER_ID_VARIABLE = 'CF_BRIDGE_WORKER_ID';
export const BRIDGE_HOST = 'laptop-bridge-staging.security.cab';
const PAGE_SIZE = 100;
const ZONE_PAGE_SIZE = 50;
const HASH = /^[a-f0-9]{64}$/;
const ID = /^[a-f0-9]{32}$/i;
const UUID = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i;
const fail = message => { throw new Error(message); };

function parseIds(value, label) {
  let ids;
  try { ids = typeof value === 'string' ? JSON.parse(value) : value; }
  catch { fail(`${label} is invalid`); }
  if (!Array.isArray(ids) || ids.some(id => typeof id !== 'string' || !ID.test(id))) fail(`${label} is missing or invalid`);
  const normalized = ids.map(id => id.toLowerCase()).sort();
  if (new Set(normalized).size !== normalized.length) fail(`${label} contains duplicates`);
  return normalized;
}

function parseReviewerIdentities(value) {
  let reviewers;
  try { reviewers = typeof value === 'string' ? JSON.parse(value) : value; }
  catch { fail('expected independent reviewer IDs are invalid'); }
  if (!Array.isArray(reviewers) || reviewers.length === 0 || reviewers.some(item =>
    !item || !['User', 'Team'].includes(item.type) || !Number.isSafeInteger(item.id) || item.id <= 0)) {
    fail('expected typed independent reviewer IDs are missing or invalid');
  }
  const normalized = reviewers.map(item => `${item.type}:${item.id}`).sort();
  if (new Set(normalized).size !== normalized.length) fail('expected typed independent reviewer IDs contain duplicates');
  return normalized;
}

export function verifyEnvironmentProtection({environment, branchPolicies, expectedReviewerIds, expectedEnvironmentName = ENVIRONMENT_NAME, dispatcherId}) {
  if (!environment || environment.name !== expectedEnvironmentName || !Array.isArray(environment.protection_rules)) {
    fail('the exact protected staging environment is absent or unverifiable');
  }
  if (environment.can_admins_bypass !== false) fail('environment API did not prove administrator bypass is disabled');
  const expected = parseReviewerIdentities(expectedReviewerIds);
  if (!Number.isSafeInteger(dispatcherId) || dispatcherId <= 0) fail('workflow dispatcher identity is unavailable');
  if (expected.length === 1 && expected[0] === `User:${dispatcherId}`) {
    fail('the workflow dispatcher is the only eligible environment reviewer');
  }
  const rules = environment.protection_rules;
  const reviewers = rules.filter(rule => rule?.type === 'required_reviewers');
  if (reviewers.length !== 1 || reviewers[0].prevent_self_review !== true || !Array.isArray(reviewers[0].reviewers)) {
    fail('staging environment reviewer protection is absent or allows self-review');
  }
  const actual = reviewers[0].reviewers.map(item => {
    const type = item?.type;
    const id = item?.reviewer?.id;
    if (!['User', 'Team'].includes(type) || !Number.isSafeInteger(id) || id <= 0) {
      fail('environment API did not return typed reviewer identities');
    }
    return `${type}:${id}`;
  }).sort();
  if (actual.length !== expected.length || actual.some((id, index) => id !== expected[index])) {
    fail('staging environment reviewers do not match the independently approved set');
  }
  const branch = environment.deployment_branch_policy;
  if (branch?.protected_branches !== false || branch?.custom_branch_policies !== true) {
    fail('staging environment is not restricted by an explicit branch policy');
  }
  const rows = branchPolicies?.branch_policies;
  if (!Array.isArray(rows) || branchPolicies.total_count !== rows.length || rows.length !== 1 ||
      rows[0]?.name !== 'main' || rows[0]?.type !== 'branch') {
    fail('staging environment API must prove exactly one branch-only main deployment rule');
  }
  return {environment: expectedEnvironmentName};
}

async function apiRequest(fetchImpl, url, token) {
  let response;
  try {
    response = await fetchImpl(url, {
      method: 'GET',
      headers: {authorization: `Bearer ${token}`, accept: 'application/json'},
      redirect: 'error',
      signal: AbortSignal.timeout(15_000),
    });
  } catch { fail('read-only provider request failed before a verified response'); }
  let body;
  try { body = await response.json(); }
  catch { fail(`read-only provider returned invalid JSON (HTTP ${response.status})`); }
  return {response, body};
}

async function cfRequest(fetchImpl, accountId, token, path, {allowMissingWorker = false} = {}) {
  const url = `${CLOUDFLARE_API}/accounts/${encodeURIComponent(accountId)}${path}`;
  const {response, body} = await apiRequest(fetchImpl, url, token);
  if (allowMissingWorker && response.status === 404) {
    const parsed = new URL(url);
    const exact = parsed.origin === new URL(CLOUDFLARE_API).origin && parsed.pathname === `${new URL(CLOUDFLARE_API).pathname}/accounts/${accountId}/workers/workers/${WORKER_NAME}` && parsed.search === '';
    if (exact && body?.success === false && (body.result === null || !Object.hasOwn(body, 'result')) &&
        Array.isArray(body.errors) && body.errors.length === 1 && body.errors[0]?.code === 10007) return null;
  }
  if (!response.ok || body?.success !== true || !Object.hasOwn(body, 'result')) {
    fail(`read-only provider response was denied or incomplete (HTTP ${response.status})`);
  }
  return {result: body.result, resultInfo: body.result_info};
}

function rowsOf(value, label) {
  const rows = Array.isArray(value) ? value : value?.items;
  if (!Array.isArray(rows)) fail(`${label} returned no verifiable list`);
  return rows;
}

async function listPages(fetchImpl, accountId, token, path, {params = {}, pageSize = PAGE_SIZE, resultPath} = {}) {
  const all = [];
  let totalCount, totalPages;
  for (let page = 1; page <= 100; page += 1) {
    const query = new URLSearchParams({...params, page: String(page), per_page: String(pageSize)});
    let response;
    if (path === '/zones') {
      const url = `${CLOUDFLARE_API}${path}?${query}`;
      const raw = await apiRequest(fetchImpl, url, token);
      if (!raw.response.ok || raw.body?.success !== true || !Object.hasOwn(raw.body, 'result')) fail(`zone list is unavailable (HTTP ${raw.response.status})`);
      response = {result: raw.body.result, resultInfo: raw.body.result_info};
    } else {
      response = await cfRequest(fetchImpl, accountId, token, `${path}?${query}`);
    }
    const listValue = resultPath ? response.result?.[resultPath] : response.result;
    const rows = rowsOf(listValue, 'provider list');
    all.push(...rows);
    const info = response.resultInfo;
    if (info !== undefined) {
      if (!info || typeof info !== 'object' || Array.isArray(info)) fail('provider pagination metadata is malformed');
      if (info.page !== undefined && info.page !== page) fail('provider pagination page did not match the request');
      if (info.per_page !== undefined && info.per_page !== pageSize) fail('provider pagination size did not match the request');
      if (info.total_count !== undefined) {
        if (!Number.isInteger(info.total_count) || info.total_count < 0) fail('provider pagination total is malformed');
        if (totalCount === undefined) totalCount = info.total_count;
        if (totalCount !== info.total_count) fail('provider list changed while paging');
      }
      if (info.total_pages !== undefined) {
        if (!Number.isInteger(info.total_pages) || info.total_pages < 0 || info.total_pages > 100) fail('provider page total is malformed');
        if (totalPages === undefined) totalPages = info.total_pages;
        if (totalPages !== info.total_pages) fail('provider list changed while paging');
      }
    }
    const byPages = totalPages !== undefined && (totalPages === 0 ? page === 1 && all.length === 0 : page >= totalPages);
    const byCount = totalCount !== undefined && all.length === totalCount;
    const byShortPage = info === undefined && rows.length < pageSize;
    if (byPages || byCount || byShortPage) {
      if (totalCount !== undefined && all.length !== totalCount) fail('provider list is incomplete');
      return all;
    }
    if (rows.length < pageSize) fail('provider list ended before its pagination total');
  }
  fail('provider pagination exceeded the hard bound');
}

async function readZones(fetchImpl, accountId, token, expectedZoneIds) {
  const zones = await listPages(fetchImpl, accountId, token, '/zones', {
    params: {'account.id': accountId, type: 'full,partial,secondary,internal'}, pageSize: ZONE_PAGE_SIZE,
  });
  const found = [];
  for (const zone of zones) {
    if (!ID.test(zone?.id ?? '') || zone?.account?.id?.toLowerCase() !== accountId.toLowerCase()) fail('account zone inventory has an invalid owner or ID');
    found.push(zone.id.toLowerCase());
  }
  found.sort();
  if (found.length !== expectedZoneIds.length || found.some((id, index) => id !== expectedZoneIds[index])) {
    fail('full account zone inventory differs from the protected expected inventory');
  }
  return zones;
}

async function readRoutes(fetchImpl, accountId, token, zones) {
  const routes = [];
  for (const zone of zones) {
    const url = `${CLOUDFLARE_API}/zones/${encodeURIComponent(zone.id)}/workers/routes`;
    const {response, body} = await apiRequest(fetchImpl, url, token);
    if (!response.ok || body?.success !== true || !Array.isArray(body.result)) fail('zone Worker route inventory is unavailable or malformed');
    routes.push(...body.result);
  }
  if (routes.some(route => route?.script === WORKER_NAME)) fail('a zone route already targets the fixed staging Worker');
  return routes;
}

function verifyVersion(version, namespaceId, expectedVersionId) {
  if (!version || version.id !== expectedVersionId || !Array.isArray(version.bindings) || !version.env ||
      typeof version.env !== 'object' || Array.isArray(version.env) || !version.exports ||
      typeof version.exports !== 'object' || !Array.isArray(version.urls)) {
    fail('exact Beta Worker version lacks its documented ID, bindings, env, exports, or URL fields');
  }
  const expectedVars = {
    ENABLED: 'false', GATEWAY_HOST: 'private-bridge-staging.invalid', GATEWAY_ACCOUNT_ID: '',
    GATEWAY_POLICY_ID: '', GATEWAY_IDENTITY_EMAIL: '', GATEWAY_POSTURE_ID: '',
    BRIDGE_HOST, ACCESS_ISSUER: process.env.CF_BRIDGE_ACCESS_ISSUER ?? '',
    ACCESS_AUDIENCE: process.env.CF_BRIDGE_ACCESS_AUDIENCE ?? '', BRIDGE_REGISTRY: '',
  };
  const bindings = version.bindings;
  if (bindings.some(binding => !binding || typeof binding.name !== 'string') ||
      new Set(bindings.map(binding => binding.name)).size !== bindings.length) fail('Beta version bindings are malformed or duplicated');
  const actualVars = Object.fromEntries(bindings.filter(binding => binding?.type === 'plain_text').map(binding => [binding.name, binding.text]));
  if (JSON.stringify(Object.entries(actualVars).sort()) !== JSON.stringify(Object.entries(expectedVars).sort())) {
    fail('deployed Worker variables are not the fixed disabled configuration');
  }
  const durable = bindings.filter(binding => binding?.type === 'durable_object_namespace');
  if (bindings.length !== Object.keys(expectedVars).length + 1 || durable.length !== 1 || durable[0]?.name !== 'BRIDGE' ||
      durable[0]?.class_name !== DO_CLASS || durable[0]?.namespace_id !== namespaceId) {
    fail('deployed Worker has an unexpected binding or the BRIDGE namespace differs');
  }
  const expectedNames = [...Object.keys(expectedVars), 'BRIDGE'].sort();
  if (bindings.length !== expectedNames.length || bindings.map(binding => binding.name).sort().some((name, index) => name !== expectedNames[index]) ||
      Object.keys(version.env).sort().some((name, index) => name !== expectedNames[index]) || Object.keys(version.env).length !== expectedNames.length) {
    fail('Beta version bindings and env aliases do not have the exact expected names');
  }
  for (const binding of bindings) {
    const alias = version.env[binding.name];
    if (!alias || alias.type !== binding.type) fail('Beta version env alias disagrees with its binding');
    for (const [key, value] of Object.entries(alias)) {
      if (key === 'name' || binding[key] !== value) fail('Beta version env alias carries conflicting binding data');
    }
    if (binding.type === 'plain_text' && Object.hasOwn(alias, 'text') && alias.text !== binding.text) {
      fail('Beta version env text differs from the binding value');
    }
  }
  const exportValue = version.exports[DO_CLASS];
  const exportKeys = Object.keys(version.exports).sort();
  if (exportKeys.length !== 1 || exportKeys[0] !== DO_CLASS || exportValue?.type !== 'durable-object' ||
      exportValue?.storage !== 'sqlite' || (exportValue?.state !== undefined && exportValue.state !== 'created')) {
    fail('deployed Worker does not declare exactly the expected SQLite export');
  }
  if (version.limits?.cpu_ms !== 10 || Object.keys(version.limits ?? {}).some(key => key !== 'cpu_ms')) fail('deployed Worker CPU limits differ from the disabled build');
  if (version.urls.length !== 0) fail('exact serving version advertises a routable URL');
  return true;
}

function verifyWorkerMetadata(worker) {
  if (!worker || worker.name !== WORKER_NAME || !ID.test(worker.id ?? '') || worker.logpush !== false ||
      worker.observability?.enabled !== false || !worker.references || !Array.isArray(worker.references.domains)) {
    fail('Worker metadata is missing or its domain or telemetry state is unverifiable');
  }
  return worker.id;
}

function exactBridgeDomains(domains, {deployed, zoneIds}) {
  if (!Array.isArray(domains)) fail('Worker domain inventory is unavailable');
  const related = domains.filter(domain => domain?.service === WORKER_NAME || domain?.hostname === BRIDGE_HOST);
  if (!deployed) {
    if (related.length !== 0) fail('empty Worker already has a custom domain or hostname collision');
    return true;
  }
  if (related.length !== 1) fail('deployed Worker must have exactly one approved custom hostname');
  const [domain] = related;
  if (domain?.service !== WORKER_NAME || domain?.hostname !== BRIDGE_HOST || domain.enabled !== true ||
      domain.previews_enabled !== false || !ID.test(domain.zone_id ?? '') ||
      !zoneIds.includes(domain.zone_id.toLowerCase())) {
    fail('Worker custom hostname, zone, or preview state is not the exact approved Access host');
  }
  return true;
}

function verifyNoGatewayAssociation(rules, workerId = null) {
  if (!Array.isArray(rules)) fail('account Gateway rule inventory is unavailable');
  for (const rule of rules) {
    if (!rule || typeof rule.action !== 'string' || typeof rule.enabled !== 'boolean') fail('account Gateway rule inventory contains an unverifiable entry');
    if (rule.action !== 'forward') continue;
    const worker = rule.rule_settings?.forward?.worker_id;
    if (typeof worker !== 'string' || worker.length === 0) fail('Gateway Forward rule lacks a verifiable Worker association');
    if (worker === WORKER_NAME || (workerId && worker === workerId)) fail('an account Gateway Forward rule targets the fixed staging Worker');
  }
  return true;
}

async function readSnapshot({accountId, token, gatewayToken, expectedZoneIds, fetchImpl = fetch}) {
  const zones = await readZones(fetchImpl, accountId, token, expectedZoneIds);
  await readRoutes(fetchImpl, accountId, token, zones);
  const gatewayRules = await listPages(fetchImpl, accountId, gatewayToken, '/gateway/rules', {pageSize: 100});
  verifyNoGatewayAssociation(gatewayRules);
  const allDomains = await listPages(fetchImpl, accountId, token, '/workers/domains');
  const allNamespaces = await listPages(fetchImpl, accountId, token, '/workers/durable_objects/namespaces', {pageSize: 1000});
  const namespaces = allNamespaces.filter(item => item?.script === WORKER_NAME);
  const worker = await cfRequest(fetchImpl, accountId, token, `/workers/workers/${WORKER_NAME}`, {allowMissingWorker: true});
  if (!worker) return {worker: null, namespaces, domains: allDomains, gatewayRules,
    zoneIds: zones.map(zone => zone.id.toLowerCase()).sort()};
  const workerId = verifyWorkerMetadata(worker.result);
  verifyNoGatewayAssociation(gatewayRules, workerId);
  const subdomain = await cfRequest(fetchImpl, accountId, token, `/workers/scripts/${WORKER_NAME}/subdomain`);
  if (subdomain.result?.enabled !== false || subdomain.result?.previews_enabled !== false) fail('Worker subdomain or preview ingress is enabled');
  const settings = await cfRequest(fetchImpl, accountId, token, `/workers/scripts/${WORKER_NAME}/script-settings`);
  if (settings.result?.logpush !== false || settings.result?.observability?.enabled !== false ||
      !Array.isArray(settings.result?.tail_consumers) || settings.result.tail_consumers.length !== 0) {
    fail('Worker logging, observability, or tail consumer settings are not explicitly disabled');
  }
  const schedules = await cfRequest(fetchImpl, accountId, token, `/workers/scripts/${WORKER_NAME}/schedules`);
  if (!Array.isArray(schedules.result?.schedules) || schedules.result.schedules.length !== 0) fail('Worker schedules are missing or non-empty');
  const deployResponse = await cfRequest(fetchImpl, accountId, token, `/workers/scripts/${WORKER_NAME}/deployments?page=1&per_page=100`, { });
  const deployments = deployResponse.result?.deployments;
  if (!Array.isArray(deployments)) fail('Worker deployment readback is missing');
  const versions=await listPages(fetchImpl,accountId,token,`/workers/scripts/${encodeURIComponent(WORKER_NAME)}/versions`,{resultPath:'items'});
  if(versions.some(version=>!version||typeof version!=='object'||!UUID.test(version.id??''))) fail('legacy Scripts version inventory contains a malformed version ID');
  if (deployments.length === 0) {
    return {worker: worker.result, workerId, subdomain: subdomain.result, settings: settings.result,
      schedules: schedules.result.schedules, namespaces, domains: allDomains, gatewayRules, version: null, versions, deployments,
      zoneIds: zones.map(zone => zone.id.toLowerCase()).sort()};
  }
  const latest = deployments[0];
  const servingVersions = latest?.versions;
  if (!Array.isArray(servingVersions) || servingVersions.length !== 1 || !UUID.test(servingVersions[0]?.version_id ?? '') ||
      servingVersions[0]?.percentage !== 100) fail('latest deployment does not identify one exact 100 percent version');
  const exactVersionId = servingVersions[0].version_id;
  const exactVersion = await cfRequest(fetchImpl, accountId, token,
    `/workers/workers/${workerId}/versions/${encodeURIComponent(exactVersionId)}`);
  if (exactVersion.result?.id !== exactVersionId) fail('Beta Worker version GET returned a different version ID');
  return {worker: worker.result, workerId, subdomain: subdomain.result, settings: settings.result,
    schedules: schedules.result.schedules, namespaces, domains: allDomains, gatewayRules, version: exactVersion.result, versions, deployments,
    zoneIds: zones.map(zone => zone.id.toLowerCase()).sort()};
}

export function verifyProviderState(snapshot, {phase, expectedZoneIds, sourceSha, expectedWorkerId}) {
  if (!['preflight', 'init-postflight', 'postflight'].includes(phase)) fail('provider verification phase is invalid');
  const expected = parseIds(expectedZoneIds, 'approved zone inventory');
  if (!snapshot || !Array.isArray(snapshot.namespaces) || !Array.isArray(snapshot.domains) || !Array.isArray(snapshot.zoneIds) ||
      snapshot.zoneIds.length !== expected.length || snapshot.zoneIds.some((id, index) => id !== expected[index])) fail('provider inventory snapshot is incomplete or has the wrong account zones');
  verifyNoGatewayAssociation(snapshot.gatewayRules, snapshot.worker?.id ?? null);
  if (!snapshot.worker) fail('the fixed Worker must be created locally by an owner before CI may proceed');
  const id = verifyWorkerMetadata(snapshot.worker);
  if (!ID.test(expectedWorkerId ?? '') || id !== expectedWorkerId.toLowerCase()) fail('Worker ID differs from the independently pinned environment identity');
  if (Array.isArray(snapshot.deployments) && snapshot.deployments.length === 0) {
    exactBridgeDomains(snapshot.domains, {deployed: false, zoneIds: expected});
    if (phase !== 'preflight' || snapshot.namespaces.length !== 0 ||
        !Array.isArray(snapshot.versions) || snapshot.versions.length !== 0 ||
        snapshot.worker.references.domains.length !== 0 ||
        snapshot.subdomain?.enabled !== false || snapshot.subdomain?.previews_enabled !== false ||
        !Array.isArray(snapshot.schedules) || snapshot.schedules.length !== 0 || !snapshot.settings ||
        snapshot.settings.logpush !== false || snapshot.settings.observability?.enabled !== false ||
        !Array.isArray(snapshot.settings.tail_consumers) || snapshot.settings.tail_consumers.length !== 0) {
      fail('only the exact empty disabled locally created Worker may enter first CI deployment');
    }
    return 'deploy';
  }
  if (snapshot.workerId !== id || snapshot.subdomain?.enabled !== false || snapshot.subdomain?.previews_enabled !== false ||
      !Array.isArray(snapshot.schedules) || snapshot.schedules.length !== 0 || !snapshot.version ||
      !Array.isArray(snapshot.deployments) || snapshot.deployments.length === 0 || snapshot.settings?.logpush !== false ||
      snapshot.settings?.observability?.enabled !== false || !Array.isArray(snapshot.settings?.tail_consumers) || snapshot.settings.tail_consumers.length !== 0) {
    fail('existing Worker is not fully verified as disabled and serving safely');
  }
  const noIngress=phase==='init-postflight'||phase==='preflight';
  exactBridgeDomains(snapshot.domains, {deployed: !noIngress, zoneIds: expected});
  if(noIngress&&snapshot.worker.references.domains.length!==0) fail('no-ingress Worker still has a reverse domain reference');
  if(noIngress&&(!Array.isArray(snapshot.versions)||snapshot.versions.length!==1||snapshot.versions[0]?.id!==snapshot.deployments[0]?.versions?.[0]?.version_id)) fail('no-ingress initialization must have exactly one version matching its sole deployment');
  if (snapshot.namespaces.length !== 1) fail('existing Worker SQLite namespace inventory is unexpected');
  const namespace = snapshot.namespaces[0];
  if (!ID.test(namespace?.id ?? '') && !UUID.test(namespace?.id ?? '')) fail('Durable Object namespace ID is invalid');
  if (namespace.script !== WORKER_NAME || namespace.class !== DO_CLASS || namespace.use_sqlite !== true) fail('Durable Object namespace owner, class, or SQLite mode is unexpected');
  const deployment = snapshot.deployments[0];
  if (!deployment || !UUID.test(deployment.id ?? '') || !Array.isArray(deployment.versions) || deployment.versions.length !== 1 ||
      deployment.versions[0]?.percentage !== 100 || !UUID.test(deployment.versions[0]?.version_id ?? '')) fail('latest serving deployment is not an exact 100 percent single-version deployment');
  if (!snapshot.version || snapshot.version.id !== deployment.versions[0].version_id) fail('exact Beta Worker version does not match the latest deployment UUID');
  verifyVersion(snapshot.version, namespace.id, deployment.versions[0].version_id);
  if (!/^[a-f0-9]{40}$/.test(sourceSha ?? '') || snapshot.version.annotations?.['workers/tag'] !== sourceSha) {
    fail('latest serving version is not tagged to this exact reviewed source SHA');
  }
  return phase === 'preflight' ? 'already-initialized-no-ingress' : noIngress ? 'verified-no-ingress' : 'verified';
}

export async function verifyBootstrapPreflight({accountId, workerId, readToken, gatewayToken, expectedZoneIds, sourceSha, fetchImpl = fetch}) {
  if (!ID.test(accountId ?? '') || typeof readToken !== 'string' || readToken.length < 20 ||
      typeof gatewayToken !== 'string' || gatewayToken.length < 20 || readToken === gatewayToken) fail('protected bootstrap account or separate read-only credentials are missing');
  const zones = parseIds(expectedZoneIds, 'approved zone inventory');
  const snapshot = await readSnapshot({accountId, token: readToken, gatewayToken, expectedZoneIds: zones, fetchImpl});
  return verifyProviderState(snapshot, {phase: 'preflight', expectedZoneIds: zones, sourceSha, expectedWorkerId: workerId});
}

export function verifyDistinctWriteCredential(readToken, gatewayToken, writeToken) {
  if (typeof readToken !== 'string' || readToken.length < 20 || typeof writeToken !== 'string' ||
      writeToken.length < 20 || typeof gatewayToken !== 'string' || gatewayToken.length < 20 ||
      new Set([readToken, gatewayToken, writeToken]).size !== 3) fail('protected Worker-read, Gateway-read, and write credentials must be separate');
  return true;
}

export async function verifyBootstrapPostflight({accountId, workerId, readToken, gatewayToken, expectedZoneIds, sourceSha, phase='postflight', fetchImpl = fetch}) {
  if (!ID.test(accountId ?? '') || typeof readToken !== 'string' || readToken.length < 20 ||
      typeof gatewayToken !== 'string' || gatewayToken.length < 20 || readToken === gatewayToken) fail('protected read-only provider identities are missing');
  const zones = parseIds(expectedZoneIds, 'approved zone inventory');
  const snapshot = await readSnapshot({accountId, token: readToken, gatewayToken, expectedZoneIds: zones, fetchImpl});
  return verifyProviderState(snapshot, {phase, expectedZoneIds: zones, sourceSha, expectedWorkerId: workerId});
}

export async function validateLocalBuild({buildOutput, mode = 'bridge'}) {
  const {readFile} = await import('node:fs/promises');
  const root = buildOutput.replace(/\/$/, '');
  const metadata = JSON.parse(await readFile(`${root}/config.json`, 'utf8'));
  if (metadata.producer?.name !== 'wrangler' || metadata.producer?.version !== '4.166.0' || metadata.buildContext?.mode !== mode) {
    fail('local Build Output producer or mode is not pinned');
  }
  const config = JSON.parse(await readFile(`${root}/workers/default/worker.config.json`, 'utf8'));
  assertEffectivePayload(config, {worker: WORKER_NAME, mode});
  return true;
}

async function githubEnvironmentGate(target) {
  const targets = {
    bootstrap: {name: BOOTSTRAP_ENVIRONMENT, reviewersVariable: 'CF_BRIDGE_RELEASE_REVIEWER_IDS'},
    release: {name: RELEASE_ENVIRONMENT, reviewersVariable: 'CF_BRIDGE_RELEASE_REVIEWER_IDS'},
  };
  const selected = targets[target];
  if (!selected) fail('environment gate target is invalid');
  const token = process.env.GH_TOKEN;
  const api = process.env.GITHUB_API_URL;
  if (!token || !api || process.env.GITHUB_REPOSITORY !== GITHUB_REPOSITORY || process.env.GITHUB_EVENT_NAME !== 'workflow_dispatch' ||
      process.env.GITHUB_REF !== 'refs/heads/main' || !/^[a-f0-9]{40}$/.test(process.env.GITHUB_SHA ?? '')) {
    fail('bootstrap dispatch is not from the fixed repository and main source');
  }
  const base = `${api.replace(/\/$/, '')}/repos/${GITHUB_REPOSITORY}/environments/${encodeURIComponent(selected.name)}`;
  const headers = {authorization: `Bearer ${token}`, accept: 'application/vnd.github+json', 'x-github-api-version': '2022-11-28'};
  const envResponse = await fetch(`${base}`, {headers, redirect: 'error', signal: AbortSignal.timeout(15_000)});
  if (!envResponse.ok) fail('protected bootstrap environment does not exist or cannot be read; deployment job was not admitted');
  const environment = await envResponse.json();
  const policyResponse = await fetch(`${base}/deployment-branch-policies?per_page=100&page=1`, {headers, redirect: 'error', signal: AbortSignal.timeout(15_000)});
  if (!policyResponse.ok) fail('protected bootstrap branch policy cannot be read');
  const policies = await policyResponse.json();
  const expected = process.env[selected.reviewersVariable];
  const result = verifyEnvironmentProtection({environment, branchPolicies: policies, expectedReviewerIds: expected,
    expectedEnvironmentName: selected.name, dispatcherId: Number(process.env.GITHUB_ACTOR_ID)});
  const outputPath = process.env.GITHUB_OUTPUT;
  if (!outputPath) fail('workflow output channel is unavailable');
  await writeFile(outputPath, `environment=${result.environment}\n`, {flag: 'a', mode: 0o600});
  console.log('existing protected environment verified; deployment job may be assigned');
}

async function providerCheck(phase) {
  const path = process.env.GITHUB_OUTPUT;
  const result = phase === 'preflight'
    ? await verifyBootstrapPreflight({accountId: process.env.CF_BRIDGE_ACCOUNT_ID, workerId: process.env.CF_BRIDGE_WORKER_ID,
      readToken: process.env.CF_BRIDGE_READ_TOKEN,
      gatewayToken: process.env.CF_BRIDGE_GATEWAY_READ_TOKEN,
      expectedZoneIds: process.env.CF_BRIDGE_ZONE_IDS, sourceSha: process.env.GITHUB_SHA})
    : await verifyBootstrapPostflight({accountId: process.env.CF_BRIDGE_ACCOUNT_ID, workerId: process.env.CF_BRIDGE_WORKER_ID,
      readToken: process.env.CF_BRIDGE_READ_TOKEN, gatewayToken: process.env.CF_BRIDGE_GATEWAY_READ_TOKEN,
      expectedZoneIds: process.env.CF_BRIDGE_ZONE_IDS,
      sourceSha: process.env.GITHUB_SHA,phase});
  if (phase === 'preflight') {
    if (!path) fail('workflow output channel is unavailable');
    await writeFile(path, `status=${result}\n`, {flag: 'a', mode: 0o600});
  }
  console.log(phase === 'preflight' ? `read-only preflight passed (${result})` : 'independent no-ingress postflight passed');
}

async function main() {
  const [command, ...args] = process.argv.slice(2);
  if (args.length !== 0) fail(`usage: staging-bootstrap.mjs <github-environment-bootstrap|github-environment-release|preflight|init-postflight|postflight|assert-distinct-credentials|validate-build>`);
  if (command === 'github-environment-bootstrap') await githubEnvironmentGate('bootstrap');
  else if (command === 'github-environment-release') await githubEnvironmentGate('release');
  else if (command === 'preflight') await providerCheck('preflight');
  else if (command === 'init-postflight') await providerCheck('init-postflight');
  else if (command === 'postflight') await providerCheck('postflight');
  else if (command === 'assert-distinct-credentials') {
    verifyDistinctWriteCredential(process.env.CF_BRIDGE_READ_TOKEN, process.env.CF_BRIDGE_GATEWAY_READ_TOKEN,
      process.env.CF_BRIDGE_API_TOKEN);
    console.log('separate read and write credentials verified');
  }
  else if (command === 'validate-build') await validateLocalBuild({buildOutput: process.env.BUILD_OUTPUT ?? '.cloudflare/output/v1',mode:process.env.BUILD_MODE??'bridge'});
  else fail('usage: staging-bootstrap.mjs <github-environment-bootstrap|github-environment-release|preflight|init-postflight|postflight|assert-distinct-credentials|validate-build>');
}

if (process.argv[1] && new URL(import.meta.url).pathname === process.argv[1]) {
  main().catch(error => { console.error(error instanceof Error ? error.message : 'staging bootstrap failed'); process.exitCode = 1; });
}
