// Tests for sync-models' parseJsonc config reader (#704).
// Run: node --test bin/sync-models.test.mjs
//
// sync-models.mjs is a top-level script (it runs the sync on import), so
// the function under test is extracted from the source and evaluated —
// the same verbatim-extraction method that reproduced #704 in triage.
// If parseJsonc's declaration shape changes, this extraction fails loudly
// rather than silently testing nothing.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const binDir = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(binDir, 'sync-models.mjs'), 'utf-8');

function extractFunction(name) {
  const start = source.indexOf(`function ${name}(`);
  assert.notEqual(start, -1, `${name} not found in sync-models.mjs`);
  const end = source.indexOf('\n}\n', start);
  assert.notEqual(end, -1, `${name} body not terminated as expected`);
  return source.slice(start, end + 3);
}

const { parseJsonc, parseJsoncTree, removeJsoncPath } = new Function(
  `${extractFunction('parseJsonc')}\n` +
  `${extractFunction('parseJsoncTree')}\n` +
  `${extractFunction('removeJsoncPath')}\n` +
  'return { parseJsonc, parseJsoncTree, removeJsoncPath };',
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

const inlineCommentCases = [
  ['after the comma', `{
  "remove": true, // keep this comment
  "keep": true
}`],
  ['after a last-member value', `{
  "remove": true // keep this comment
}`],
  ['between the value and comma', `{
  "remove": true /* keep this comment */,
  "keep": true
}`],
  ['before a comma on the next line', `{
  "remove": true // keep this comment
  ,
  "keep": true
}`],
];

for (const [placement, text] of inlineCommentCases) {
  test(`refuses to remove a member with an inline comment ${placement}`, () => {
    assert.throws(
      () => removeJsoncPath(text, ['remove']),
      error => error.name === 'JsoncCommentRemovalError' &&
        /remove.*inline comment/i.test(error.message),
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

test('a non-numeric version capture is excluded deterministically in both input orders (#6893)', () => {
  // Oracle: permutation invariance — the returned set must not depend
  // on enumeration order, and a NaN-component capture never wins.
  const policy = { families: { suffixed: '^model-([0-9]+-[a-z0-9]+)$' } };
  const forward = allowedModelIds(policy, { keep: ['suffixed'] }, ['model-2-1', 'model-2-beta']);
  const reversed = allowedModelIds(policy, { keep: ['suffixed'] }, ['model-2-beta', 'model-2-1']);
  assert.deepEqual(forward, new Set(['model-2-1']));
  assert.deepEqual(reversed, forward);
});

test('a family whose only candidates have non-numeric captures selects nothing (#6893)', () => {
  const policy = { families: { suffixed: '^model-([0-9]+-[a-z0-9]+)$' } };
  assert.deepEqual(
    allowedModelIds(policy, { keep: ['suffixed'] }, ['model-2-beta', 'model-2-alpha']),
    new Set(),
  );
  assert.deepEqual(
    allowedModelIds(policy, { keep: ['suffixed'] }, ['model-2-alpha', 'model-2-beta']),
    new Set(),
  );
});

test('allowedModelIds returns the same set for every permutation of the same ids (#6893)', () => {
  const policy = { families: { suffixed: '^model-([0-9]+-[a-z0-9]+)$' } };
  const ids = ['model-2-1', 'model-2-beta', 'model-3-0'];
  const permutations = [
    ids,
    [ids[0], ids[2], ids[1]],
    [ids[1], ids[0], ids[2]],
    [ids[1], ids[2], ids[0]],
    [ids[2], ids[0], ids[1]],
    [ids[2], ids[1], ids[0]],
  ];
  for (const permutation of permutations) {
    assert.deepEqual(
      allowedModelIds(policy, { keep: ['suffixed'] }, permutation),
      new Set(['model-3-0']),
    );
  }
});

test('shipped-policy control: families select the same newest model in both input orders (#6893)', () => {
  const policyPath = path.join(binDir, 'sync-models.policy.json');
  const shippedPolicy = JSON.parse(fs.readFileSync(policyPath, 'utf-8'));
  const ids = ['gemini-1-flash', 'gemini-2-flash', 'gemini-1.5-flash'];
  const forward = allowedModelIds(shippedPolicy, { keep: ['gemini-flash'] }, ids);
  const reversed = allowedModelIds(shippedPolicy, { keep: ['gemini-flash'] }, [...ids].reverse());
  assert.deepEqual(forward, new Set(['gemini-2-flash']));
  assert.deepEqual(reversed, forward);
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

test('removes a comment-free member and leaves parseable JSONC', () => {
  const output = removeJsoncPath(`{
  "remove": true,
  "keep": true
}`, ['remove']);
  assert.deepEqual(parseJsonc(output), { keep: true });
  assert.equal(parseJsoncTree(output).members.length, 1);
});

test('keeps a standalone comment when removing the following member', () => {
  const output = removeJsoncPath(`{
  // standalone note
  "remove": true,
  "keep": true
}`, ['remove']);
  assert.match(output, /\/\/ standalone note/);
  assert.deepEqual(parseJsonc(output), { keep: true });
});

// Strict argv validation (#3297): unknown arguments are a usage error,
// reported before any fetch/discovery/write. These probes run the real
// script with an isolated HOME and an explicit OPENCODE_CONFIG_PATH so the
// temp target config — never the live config — is the only write candidate.
const scriptPath = path.join(binDir, 'sync-models.mjs');

function runWithTempConfig(args) {
  const tmpHome = fs.mkdtempSync(path.join(os.tmpdir(), 'sync-models-test-home-'));
  const configPath = path.join(tmpHome, 'opencode.jsonc');
  const before = '{\n  // temp probe config\n  "provider": {}\n}\n';
  fs.writeFileSync(configPath, before);
  const result = spawnSync(process.execPath, [scriptPath, ...args], {
    encoding: 'utf-8',
    timeout: 60000,
    env: {
      ...process.env,
      HOME: tmpHome,
      OPENCODE_CONFIG_PATH: configPath,
    },
  });
  const after = fs.readFileSync(configPath, 'utf-8');
  const backupExists = fs.existsSync(`${configPath}.backup`);
  fs.rmSync(tmpHome, { recursive: true, force: true });
  return { result, before, after, backupExists };
}

test('rejects --bogus-flag non-zero and leaves the target config byte-identical', () => {
  const { result, before, after, backupExists } = runWithTempConfig(['--bogus-flag']);
  assert.notEqual(result.status, 0);
  assert.match(`${result.stderr}${result.stdout}`, /--bogus-flag/);
  assert.equal(after, before);
  assert.equal(backupExists, false);
});

test('rejects typo --dry-rnu non-zero, names it, and leaves the target config byte-identical', () => {
  const { result, before, after, backupExists } = runWithTempConfig(['--dry-rnu']);
  assert.notEqual(result.status, 0);
  assert.match(`${result.stderr}${result.stdout}`, /--dry-rnu/);
  assert.equal(after, before);
  assert.equal(backupExists, false);
});

test('rejects a positional argument non-zero and leaves the target config byte-identical', () => {
  const { result, before, after, backupExists } = runWithTempConfig(['some-positional']);
  assert.notEqual(result.status, 0);
  assert.match(`${result.stderr}${result.stdout}`, /some-positional/);
  assert.equal(after, before);
  assert.equal(backupExists, false);
});

test('--dry-run still exits 0 writing nothing', () => {
  const { result, before, after, backupExists } = runWithTempConfig(['--dry-run']);
  assert.equal(result.status, 0);
  assert.equal(after, before);
  assert.equal(backupExists, false);
});