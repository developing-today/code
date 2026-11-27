import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  buildProbePayload,
  classifyResult,
  createProbeBudget,
  DIRECT_ADAPTERS,
  DIRECT_PROBE_MAX_CALLS,
  DIRECT_PROBE_MAX_OUTPUT_TOKENS,
  extractFreeLeaves,
  parseArgs,
  ProbeError,
  PROBE_SENTINEL,
  resolveDirectAdapter,
  runDirectProbe,
  tagContextSource,
  UsageError,
} from './free-context-probe.mjs';
import { parseJsonc } from './provider-catalog-sync.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const script = fileURLToPath(new URL('./free-context-probe.mjs', import.meta.url));

test('free leaf inventory is sourced from configured plan targets and excludes route wrappers', () => {
  const config = parseJsonc(fs.readFileSync(`${root}/.opencode/opencode.jsonc`, 'utf8'));
  const { leaves, missingPlans } = extractFreeLeaves(config);
  assert.deepEqual(missingPlans, [], 'oracle: every curated free plan id must exist in model_routes');
  assert.equal(leaves.length, 119, 'oracle: current leaf count after expanding configured provider-route aliases');
  assert.ok(leaves.some((leaf) => leaf.target === 'google-free/gemini-4-hydrogen#xhigh'));
  assert.ok(leaves.some((leaf) => leaf.provider === 'google.muse' && leaf.model === 'gemini-4-hydrogen'));
  assert.ok(leaves.some((leaf) => leaf.provider === 'opencode.drewrypope' && leaf.model === 'beam-free'));
  assert.ok(leaves.some((leaf) => leaf.target === 'kilo/kilo-auto/free'));
  assert.ok(leaves.every((leaf) => leaf.provider !== 'opencode-route'));
});

test('probe payload is deterministic and terminates in its sentinel', () => {
  const first = buildProbePayload({ blocks: 4 });
  const second = buildProbePayload({ blocks: 4 });
  assert.equal(first, second, 'oracle: exact byte equality for identical block count');
  assert.ok(first.endsWith(`${PROBE_SENTINEL}\n`), 'oracle: planted end-of-prompt sentinel is last');
});

test('result classifier distinguishes context failure from rate/auth walls', () => {
  assert.deepEqual(classifyResult({ httpStatus: 413, bodyText: '' }), { outcome: 'fail-other', reason: 'http-413' });
  assert.deepEqual(classifyResult({ httpStatus: 400, bodyText: 'context_length_exceeded' }), { outcome: 'fail-context', reason: 'context-signal' });
  assert.deepEqual(classifyResult({ httpStatus: 400, bodyText: 'max_tokens reached' }), { outcome: 'fail-other', reason: 'http-400' });
  assert.deepEqual(classifyResult({ httpStatus: 429, bodyText: 'quota' }), { outcome: 'fail-rate', reason: 'http-429' });
  assert.deepEqual(classifyResult({ httpStatus: 401, bodyText: 'invalid key' }), { outcome: 'fail-auth', reason: 'http-401' });
});

test('context provenance is never promoted without an empirical pass', () => {
  assert.equal(tagContextSource({ empiricallyTested: false }), 'provider-published');
  assert.equal(tagContextSource({ empiricallyTested: true }), 'empirically-tested');
});

test('CLI rejects unknown, duplicate and incompatible flags before network', () => {
  assert.throws(() => parseArgs(['--live', '--leef', 'groq/model']), UsageError);
  assert.throws(() => parseArgs(['--live', '--leaf']), UsageError);
  assert.throws(() => parseArgs(['--live', '--live']), UsageError);
});

test('CLI rejects live plus inventory mode before reading config or making requests', () => {
  const result = spawnSync(process.execPath, [script, '--live', '--inventory-only'], { cwd: root, encoding: 'utf8' });
  assert.equal(result.status, 2);
  assert.match(result.stderr, /mutually exclusive/);
});

test('inventory-only CLI reports configured direct leaves without network calls', () => {
  const result = spawnSync(process.execPath, [script, '--inventory-only'], { cwd: root, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  const inventory = JSON.parse(result.stdout);
  assert.equal(inventory.leafCount, 119);
  assert.ok(inventory.leaves.every((leaf) => leaf.contextSource === 'provider-published'));
});

// ---------------------------------------------------------------------------
// Slice 4 (task 26036): credentialed direct adapters. Oracle for every test:
// fail closed BEFORE any network call — the mock fetch counts its calls and
// any fail-closed path must leave that count at zero.
// ---------------------------------------------------------------------------
const GROQ_LEAF = { provider: 'groq', model: 'openai/gpt-oss-120b' };
const WIRED_LEAVES = [GROQ_LEAF, { provider: 'nvidia', model: 'nvidia/nemotron-3-ultra-550b-a55b' }];

function mockJsonFetch(payload, { status = 200 } = {}) {
  const calls = [];
  const fetchImpl = async (url, init) => {
    calls.push({ url, init });
    const bytes = new TextEncoder().encode(JSON.stringify(payload));
    let sent = false;
    return {
      status,
      body: {
        getReader: () => ({
          read: async () => {
            if (sent) return { done: true };
            sent = true;
            return { value: bytes, done: false };
          },
          cancel: async () => {},
        }),
      },
    };
  };
  return { fetchImpl, calls };
}

test('direct adapter registry covers the source-backed non-Kilo families and excludes keyless Kilo', () => {
  assert.deepEqual(Object.keys(DIRECT_ADAPTERS).sort(), ['groq', 'nvidia', 'openrouter']);
  for (const adapter of Object.values(DIRECT_ADAPTERS)) {
    assert.ok(adapter.endpoint.startsWith('https://') && adapter.endpoint.endsWith('/chat/completions'), 'oracle: public OpenAI-compatible chat endpoint');
    assert.match(adapter.credentialEnv, /^[A-Z][A-Z0-9_]*$/, 'oracle: credential is named as an env var, never a value');
    assert.ok(adapter.source.length > 0, 'oracle: every adapter row carries its source');
  }
  assert.equal(DIRECT_ADAPTERS.kilo, undefined, 'oracle: kilo stays on its separate keyless path');
  assert.equal(DIRECT_PROBE_MAX_CALLS, 5);
  assert.equal(DIRECT_PROBE_MAX_OUTPUT_TOKENS, 64);
});

test('direct adapter fails closed on an unsupported provider before network', async () => {
  const { fetchImpl, calls } = mockJsonFetch({});
  await assert.rejects(
    runDirectProbe({
      leaf: { provider: 'zai', model: 'glm-4.7-flash' },
      leaves: WIRED_LEAVES,
      prompt: 'x',
      timeoutMs: 5000,
      env: { ZAI_API_KEY: 'test-only' },
      fetchImpl,
      budget: createProbeBudget({ maxCalls: 1 }),
    }),
    (err) => err instanceof ProbeError && err.code === 'unsupported-provider',
  );
  assert.equal(calls.length, 0, 'oracle: fail-closed means zero network calls');
});

test('direct adapter fails closed on a model outside the wired inventory before network', async () => {
  const { fetchImpl, calls } = mockJsonFetch({});
  await assert.rejects(
    runDirectProbe({
      leaf: { provider: 'groq', model: 'not/a-wired-model' },
      leaves: WIRED_LEAVES,
      prompt: 'x',
      timeoutMs: 5000,
      env: { GROQ_API_KEY: 'test-only' },
      fetchImpl,
      budget: createProbeBudget({ maxCalls: 1 }),
    }),
    (err) => err instanceof ProbeError && err.code === 'unsupported-model',
  );
  assert.equal(calls.length, 0, 'oracle: fail-closed means zero network calls');
});

test('direct adapter fails closed when the named credential is missing before network', async () => {
  const { fetchImpl, calls } = mockJsonFetch({});
  await assert.rejects(
    runDirectProbe({
      leaf: GROQ_LEAF,
      leaves: WIRED_LEAVES,
      prompt: 'x',
      timeoutMs: 5000,
      env: {},
      fetchImpl,
      budget: createProbeBudget({ maxCalls: 1 }),
    }),
    (err) => err instanceof ProbeError && err.code === 'missing-credentials' && /GROQ_API_KEY/.test(err.message),
  );
  assert.equal(calls.length, 0, 'oracle: fail-closed means zero network calls');
  assert.throws(
    () => resolveDirectAdapter({ leaf: GROQ_LEAF, leaves: WIRED_LEAVES, env: { GROQ_API_KEY: '' } }),
    (err) => err instanceof ProbeError && err.code === 'missing-credentials',
    'oracle: an empty credential value is treated as missing',
  );
});

test('direct adapter enforces the output cap before and after the call', async () => {
  const { fetchImpl, calls } = mockJsonFetch({});
  await assert.rejects(
    runDirectProbe({
      leaf: GROQ_LEAF,
      leaves: WIRED_LEAVES,
      prompt: 'x',
      timeoutMs: 5000,
      maxOutputTokens: DIRECT_PROBE_MAX_OUTPUT_TOKENS + 1,
      env: { GROQ_API_KEY: 'test-only' },
      fetchImpl,
      budget: createProbeBudget({ maxCalls: 1 }),
    }),
    (err) => err instanceof ProbeError && err.code === 'output-cap-violation',
  );
  assert.equal(calls.length, 0, 'oracle: a cap above the hard cap is refused before network');

  const over = mockJsonFetch({
    model: 'openai/gpt-oss-120b',
    usage: { prompt_tokens: 271, completion_tokens: DIRECT_PROBE_MAX_OUTPUT_TOKENS + 36 },
    choices: [{ message: { content: PROBE_SENTINEL }, finish_reason: 'length' }],
  });
  const result = await runDirectProbe({
    leaf: GROQ_LEAF,
    leaves: WIRED_LEAVES,
    prompt: 'x',
    timeoutMs: 5000,
    env: { GROQ_API_KEY: 'test-only' },
    fetchImpl: over.fetchImpl,
    budget: createProbeBudget({ maxCalls: 1 }),
  });
  assert.equal(result.outcome, 'fail-cap', 'oracle: reported output beyond the cap invalidates the measurement');
  assert.equal(result.reason, 'output-cap-violation');
  assert.equal(over.calls.length, 1);
});

test('probe budget is hard-bounded at five calls and fails closed when exhausted', async () => {
  assert.throws(
    () => createProbeBudget({ maxCalls: DIRECT_PROBE_MAX_CALLS + 1 }),
    (err) => err instanceof ProbeError && err.code === 'call-budget-violation',
    'oracle: no run may budget more than the slice hard bound of 5 live calls',
  );
  const budget = createProbeBudget({ maxCalls: 1 });
  const ok = mockJsonFetch({
    model: 'openai/gpt-oss-120b',
    usage: { prompt_tokens: 10, completion_tokens: 4 },
    choices: [{ message: { content: PROBE_SENTINEL }, finish_reason: 'stop' }],
  });
  await runDirectProbe({
    leaf: GROQ_LEAF,
    leaves: WIRED_LEAVES,
    prompt: 'x',
    timeoutMs: 5000,
    env: { GROQ_API_KEY: 'test-only' },
    fetchImpl: ok.fetchImpl,
    budget,
  });
  await assert.rejects(
    runDirectProbe({
      leaf: GROQ_LEAF,
      leaves: WIRED_LEAVES,
      prompt: 'x',
      timeoutMs: 5000,
      env: { GROQ_API_KEY: 'test-only' },
      fetchImpl: ok.fetchImpl,
      budget,
    }),
    (err) => err instanceof ProbeError && err.code === 'call-budget-exceeded',
  );
  assert.equal(ok.calls.length, 1, 'oracle: the exhausted budget blocks the second call before network');
});

test('direct adapter happy path records serving leaf and credential source by name only', async () => {
  const { fetchImpl, calls } = mockJsonFetch({
    model: 'openai/gpt-oss-120b',
    provider: 'Groq',
    usage: { prompt_tokens: 271, completion_tokens: 12 },
    choices: [{ message: { content: PROBE_SENTINEL }, finish_reason: 'stop' }],
  });
  const result = await runDirectProbe({
    leaf: GROQ_LEAF,
    leaves: WIRED_LEAVES,
    prompt: `Repeat the final sentinel exactly.\n${PROBE_SENTINEL}`,
    timeoutMs: 5000,
    env: { GROQ_API_KEY: 'test-only-value' },
    fetchImpl,
    budget: createProbeBudget({ maxCalls: 1 }),
  });
  assert.equal(result.outcome, 'pass');
  assert.equal(result.sentinelReturned, true);
  assert.equal(result.servingLeaf, 'groq/openai/gpt-oss-120b', 'oracle: serving leaf is provider + response-reported model');
  assert.equal(result.credentialSource, 'env:GROQ_API_KEY', 'oracle: records name the credential source, never the value');
  assert.ok(!JSON.stringify(result).includes('test-only-value'), 'oracle: no credential value appears in the result record');
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, DIRECT_ADAPTERS.groq.endpoint);
  const sentBody = JSON.parse(calls[0].init.body);
  assert.equal(sentBody.model, 'openai/gpt-oss-120b');
  assert.equal(sentBody.max_tokens, DIRECT_PROBE_MAX_OUTPUT_TOKENS, 'oracle: the request itself carries the strict output cap');
  assert.equal(calls[0].init.headers.authorization, 'Bearer test-only-value', 'oracle: the credential travels only in the auth header');
});

test('CLI live direct probe fails closed with no credential and writes no record', () => {
  const env = { ...process.env };
  delete env.GROQ_API_KEY;
  const result = spawnSync(process.execPath, [script, '--live', '--leaf', 'groq/openai/gpt-oss-120b', '--max-probes', '1'], {
    cwd: root,
    encoding: 'utf8',
    env,
  });
  assert.equal(result.status, 4, result.stderr);
  assert.match(result.stderr, /missing-credentials/);
  assert.match(result.stderr, /GROQ_API_KEY/);
  assert.equal(result.stdout, '', 'oracle: a probe that never ran produces no record');
});
