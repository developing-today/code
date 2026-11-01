// Tests for sync-models' parseJsonc config reader (#704).
// Run: node --test bin/sync-models.test.mjs
//
// sync-models.mjs is a top-level script (it runs the sync on import), so
// the function under test is extracted from the source and evaluated —
// the same verbatim-extraction method that reproduced #704 in triage.
// If parseJsonc's declaration shape changes, this extraction fails loudly
// rather than silently testing nothing.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const binDir = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(binDir, 'sync-models.mjs'), 'utf-8');

const start = source.indexOf('function parseJsonc(text) {');
assert.notEqual(start, -1, 'parseJsonc not found in sync-models.mjs');
const end = source.indexOf('\n}\n', start);
assert.notEqual(end, -1, 'parseJsonc body not terminated as expected');
const parseJsonc = new Function(
  `${source.slice(start, end + 3)}\nreturn parseJsonc;`,
)();

const loadPolicyStart = source.indexOf('function loadPolicy() {');
assert.notEqual(loadPolicyStart, -1, 'loadPolicy not found in sync-models.mjs');
const loadPolicyEnd = source.indexOf('\n}\n', loadPolicyStart);
assert.notEqual(loadPolicyEnd, -1, 'loadPolicy body not terminated as expected');
const loadPolicyFactory = new Function(
  'fs',
  'POLICY_PATH',
  `${source.slice(loadPolicyStart, loadPolicyEnd + 3)}\nreturn loadPolicy;`,
);

const compareVersionsStart = source.indexOf('const compareVersions = (a, b) => {');
assert.notEqual(compareVersionsStart, -1, 'compareVersions not found in sync-models.mjs');
const allowedModelIdsStart = source.indexOf('function allowedModelIds(');
assert.notEqual(allowedModelIdsStart, -1, 'allowedModelIds not found in sync-models.mjs');
const allowedModelIdsEnd = source.indexOf('\n}\n', allowedModelIdsStart);
assert.notEqual(allowedModelIdsEnd, -1, 'allowedModelIds body not terminated as expected');
const allowedModelIds = new Function(
  `${source.slice(compareVersionsStart, allowedModelIdsEnd + 3)}\nreturn allowedModelIds;`,
)();

const loadPolicyFrom = policy => loadPolicyFactory(
  { existsSync: () => true, readFileSync: () => JSON.stringify(policy) },
  'test-policy.json',
)();

test('parses a config with a multi-line block comment', () => {
  const config = parseJsonc(`{
    /* multi
       line
       comment */
    "a": 1
  }`);
  assert.deepEqual(config, { a: 1 });
});

test('parses a string containing " // " verbatim', () => {
  const config = parseJsonc(`{
    "url": "https://example.com // path",
    "a": 1
  }`);
  assert.equal(config.url, 'https://example.com // path');
  assert.equal(config.a, 1);
});

test('parses a string containing ", ]" without corrupting it', () => {
  const config = parseJsonc(`{
    "note": "x, ]",
    "a": 1
  }`);
  assert.equal(config.note, 'x, ]');
});

test('parses a string containing ", }" without corrupting it', () => {
  const config = parseJsonc(`{ "note": "x, }", "a": 1 }`);
  assert.equal(config.note, 'x, }');
});

test('still tolerates trailing commas and // line comments', () => {
  const config = parseJsonc(`{
    // a line comment
    "a": 1,
    "list": [1, 2,],
  }`);
  assert.deepEqual(config, { a: 1, list: [1, 2] });
});

test('still throws on a genuinely malformed config', () => {
  assert.throws(() => parseJsonc('{ "a": }'));
  assert.throws(() => parseJsonc('{ "a": 1'));
});

for (const family of ['constructor', '__proto__', 'toString', 'hasOwnProperty']) {
  test(`rejects inherited family name ${JSON.stringify(family)} in provider keep list`, () => {
    const policy = {
      families: { known: '^model-(\\d+)$' },
      providers: { example: { keep: [family] } },
    };
    assert.throws(
      () => loadPolicyFrom(policy),
      error => error.message === `sync-models.policy.json: example keeps unknown family "${family}"`,
    );
  });
}

test('a family pattern without capture group 1 does not select an id or throw', () => {
  const policy = { families: { plain: '^plain-model$' } };
  assert.deepEqual(
    allowedModelIds(policy, { keep: ['plain'] }, ['plain-model']),
    new Set(),
  );
});

test('a non-participating optional capture does not prevent participating ids from selection', () => {
  const policy = { families: { versioned: '^(?:model-(\\d+)|stable)$' } };
  assert.deepEqual(
    allowedModelIds(policy, { keep: ['versioned'] }, ['stable', 'model-2']),
    new Set(['model-2']),
  );
});

test('shipped policy still loads unchanged and known families select the newest model', () => {
  const policyPath = path.join(binDir, 'sync-models.policy.json');
  const shippedPolicy = JSON.parse(fs.readFileSync(policyPath, 'utf-8'));
  const loadShippedPolicy = loadPolicyFactory(fs, policyPath);
  const loadedPolicy = loadShippedPolicy();

  assert.deepEqual(loadedPolicy, shippedPolicy);
  assert.deepEqual(
    allowedModelIds(
      loadedPolicy,
      { keep: ['gemini-flash'] },
      ['gemini-1-flash', 'gemini-2-flash'],
    ),
    new Set(['gemini-2-flash']),
  );
});
