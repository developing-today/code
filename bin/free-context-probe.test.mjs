import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  buildProbePayload,
  classifyResult,
  extractFreeLeaves,
  parseArgs,
  PROBE_SENTINEL,
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
