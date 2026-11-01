import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {
  parseJsonc,
  findModelsSpan,
  insertModelEntry,
  classify,
  prettifyModelName,
  makeEntry,
  applyAdditions,
  assertCatalogOnly,
  catalogSyncBranch,
  assertNotDefaultBranch,
  openPrCommands,
  runSync,
} from './provider-catalog-sync.mjs';

const FIXTURE = `{
  // fleet config fixture
  "disabled_providers": ["groq"],
  "provider": {
    "proxy-a": {
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "http://127.0.0.1:18744/v1" },
      "models": {
        "gpt-1-sol": {
          "name": "GPT-1 Sol (codex)",
          "limit": { "context": 1000, "output": 100 },
          "variants": { "low": { "options": { "reasoning_effort": "low" } } }
        },
        "gpt-1-luna": { "name": "GPT-1 Luna (codex)", "variants": { "low": { "options": { "reasoning_effort": "low" } } } }
      }
    },
    "name-only-b": {
      "npm": "@ai-sdk/openai-compatible",
      "models": {
        "acme/turbo-2": { "name": "Acme Turbo 2 (Door B)" },
        "acme/turbo-1": { "name": "Acme Turbo 1 (Door B)" }
      }
    }
  },
  "experimental": { "model_routes": { "workhorse": { "targets": ["proxy-a/gpt-1-sol"] } } }
}
`;

const runSnapshot = async (snapshot) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'provider-catalog-sync-'));
  const sourceDir = path.join(dir, 'sources');
  fs.mkdirSync(sourceDir);
  const configPath = path.join(dir, 'config.jsonc');
  fs.writeFileSync(configPath, JSON.stringify({
    provider: {
      'claude-code': { models: { 'model-a': {}, 'model-b': {} } },
    },
  }));
  fs.writeFileSync(path.join(sourceDir, '18749.json'), JSON.stringify(snapshot));
  try {
    return await runSync({ configPath, modelsDevSpec: null, sourceDir });
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
};

for (const [name, snapshot] of [
  ['missing data', { unexpected: [] }],
  ['non-array data', { data: 'oops' }],
  ['entry without an id', { data: [{ name: 'no-id' }] }],
  ['null entry', { data: [null] }],
]) {
  test(`runSync marks proxy snapshot with ${name} as not-enumerable`, async () => {
    const { report } = await runSnapshot(snapshot);
    const entry = report.providers['claude-code'];
    assert.equal(entry.status, 'not-enumerable');
    assert.match(entry.note, /proxy :18749 enumeration failed:/);
    assert.equal(entry.live_count, undefined);
    assert.equal(entry.vanishings_basis, undefined);
  });
}

test('runSync preserves the valid proxy snapshot report', async () => {
  const { report } = await runSnapshot({ data: [{ id: 'model-a' }, { id: 'model-b' }] });
  const entry = report.providers['claude-code'];
  assert.equal(entry.live_count, 2);
  assert.deepEqual(entry.additions_validated, []);
  assert.deepEqual(entry.vanishings, []);
  assert.equal(entry.vanishings_basis, 'live');
});

test('parseJsonc strips comments and trailing commas', () => {
  const cfg = parseJsonc(FIXTURE);
  assert.deepEqual(Object.keys(cfg.provider), ['proxy-a', 'name-only-b']);
  assert.equal(cfg.provider['proxy-a'].models['gpt-1-sol'].limit.context, 1000);
});

test('findModelsSpan locates the models object per provider', () => {
  const span = findModelsSpan(FIXTURE, 'name-only-b');
  assert.ok(span);
  assert.equal(span.memberCount, 2);
  assert.equal(findModelsSpan(FIXTURE, 'missing-provider'), null);
});

test('insertModelEntry adds an entry and the file still parses', () => {
  const out = insertModelEntry(FIXTURE, 'name-only-b', 'acme/turbo-3', { name: 'Acme Turbo 3 (Door B)' });
  const cfg = parseJsonc(out);
  assert.deepEqual(Object.keys(cfg.provider['name-only-b'].models), ['acme/turbo-2', 'acme/turbo-1', 'acme/turbo-3']);
  assert.equal(cfg.provider['name-only-b'].models['acme/turbo-3'].name, 'Acme Turbo 3 (Door B)');
  // untouched provider is byte-identical in parsed form
  assert.deepEqual(cfg.provider['proxy-a'], parseJsonc(FIXTURE).provider['proxy-a']);
  assert.deepEqual(cfg.experimental, parseJsonc(FIXTURE).experimental);
});

test('classify compares proxy lists raw (rung ids are catalog entries too)', () => {
  // The fleet catalog mirrors served lists verbatim (#1304 convention):
  // gpt-1-sol-xhigh is a real catalog id wherever the door serves it as one.
  const r = classify({
    configIds: ['gpt-1-sol', 'gpt-1-sol-xhigh', 'deliberate-extra'],
    liveIds: ['gpt-1-sol', 'gpt-1-sol-xhigh', 'gpt-1-sol-low'],
    crosscheckIds: null,
  });
  assert.deepEqual(r.additions_validated, ['gpt-1-sol-low']);
  assert.deepEqual(r.vanishings, ['deliberate-extra']);
  assert.equal(r.vanishings_basis, 'live');
});

test('classify separates validated, unverified, vanishings', () => {
  const r = classify({
    configIds: ['a', 'b', 'deliberate-extra'],
    liveIds: ['a', 'b', 'new-live'],
    crosscheckIds: ['a', 'b', 'new-live', 'catalog-only'],
  });
  assert.deepEqual(r.additions_validated, ['new-live']);
  assert.deepEqual(r.additions_unverified, ['catalog-only']);
  assert.deepEqual(r.vanishings, ['deliberate-extra']);
  assert.deepEqual(r.unknown_models, []);
});

test('classify marks additions missing from models.dev as unknown (research trail)', () => {
  const r = classify({ configIds: [], liveIds: ['mystery-1'], crosscheckIds: ['other'] });
  assert.deepEqual(r.additions_validated, ['mystery-1']);
  assert.deepEqual(r.unknown_models, ['mystery-1']);
});

test('classify without a live list keeps every addition unverified', () => {
  const r = classify({ configIds: ['a'], liveIds: null, crosscheckIds: ['a', 'b'] });
  assert.deepEqual(r.additions_validated, []);
  assert.deepEqual(r.additions_unverified, ['b']);
  assert.deepEqual(r.vanishings, []);
  assert.equal(r.vanishings_basis, 'crosscheck');
});

test('prettifyModelName follows sibling display conventions', () => {
  assert.equal(prettifyModelName('deepseek/deepseek-v4-pro'), 'DeepSeek v4 Pro');
  assert.equal(prettifyModelName('gpt-6.2-sol'), 'GPT 6.2 Sol');
  assert.equal(prettifyModelName('claude-haiku-5-5'), 'Claude Haiku 5.5');
});

test('makeEntry: name-only block yields a name-only entry with the block suffix', () => {
  const siblings = parseJsonc(FIXTURE).provider['name-only-b'].models;
  const e = makeEntry('name-only-b', 'acme/turbo-3', siblings, null);
  assert.deepEqual(e, { name: 'Acme Turbo 3 (Door B)' });
});

test('makeEntry: variant block clones the nearest sibling and prefers models.dev limits', () => {
  const siblings = parseJsonc(FIXTURE).provider['proxy-a'].models;
  const e = makeEntry('proxy-a', 'gpt-1-terra', siblings, { limit: { context: 2000, output: 200 } });
  assert.equal(e.name, 'GPT 1 Terra (codex)');
  assert.deepEqual(e.limit, { context: 2000, output: 200 });
  assert.deepEqual(e.variants, { low: { options: { reasoning_effort: 'low' } } });
});

test('insertModelEntry matches the file indentation style', () => {
  const out = insertModelEntry(FIXTURE, 'proxy-a', 'gpt-1-terra', {
    name: 'GPT 1 Terra',
    variants: { low: { options: { reasoning_effort: 'low' } } },
  });
  const siblingName = FIXTURE.split('\n').find((l) => l.includes('"name": "GPT-1 Sol'));
  const insertedName = out.split('\n').find((l) => l.includes('"name": "GPT 1 Terra'));
  assert.equal(insertedName.match(/^\s*/)[0], siblingName.match(/^\s*/)[0]);
});

test('applyAdditions + assertCatalogOnly: only catalog entries change', () => {
  const cfg = parseJsonc(FIXTURE);
  const report = {
    providers: {
      'proxy-a': { additions_validated: ['gpt-1-terra'] },
      'name-only-b': { additions_validated: [] },
    },
  };
  const { text, added } = applyAdditions(FIXTURE, cfg, report, null);
  assert.deepEqual(added, { 'proxy-a': ['gpt-1-terra'] });
  assertCatalogOnly(FIXTURE, text, added); // must not throw
  const after = parseJsonc(text);
  assert.ok(after.provider['proxy-a'].models['gpt-1-terra']);
  // guard actually bites when non-catalog content moves
  const tampered = text.replace('"groq"', '"groq", "meta"');
  assert.throws(() => assertCatalogOnly(FIXTURE, tampered, added), /catalog-only guard/);
});

test('catalogSyncBranch is the dated dedicated branch', () => {
  assert.equal(catalogSyncBranch('2026-10-28'), 'chore/catalog-sync-2026-10-28');
});

test('openPrCommands: branch step precedes commit, push sets upstream, gh gets --head', () => {
  const branch = 'chore/catalog-sync-2026-10-28';
  const cmds = openPrCommands({
    branch,
    branchExists: false,
    date: '2026-10-28',
    bodyFile: '/tmp/x/pr-body.md',
  });
  assert.deepEqual(cmds[0], ['git', 'checkout', '-b', branch]);
  const step = (name) => cmds.findIndex((c) => c[1] === name);
  assert.ok(step('checkout') < step('commit'), 'branch step precedes the commit');
  const push = cmds.find((c) => c[0] === 'git' && c[1] === 'push');
  assert.deepEqual(push, ['git', 'push', '--set-upstream', 'origin', branch]);
  const gh = cmds.find((c) => c[0] === 'gh');
  assert.equal(gh[gh.indexOf('--head') + 1], branch);
});

test('openPrCommands: same-day rerun checks out the existing branch', () => {
  const branch = 'chore/catalog-sync-2026-10-28';
  const cmds = openPrCommands({
    branch,
    branchExists: true,
    date: '2026-10-28',
    bodyFile: '/tmp/x/pr-body.md',
  });
  assert.deepEqual(cmds[0], ['git', 'checkout', branch]);
});

test('assertNotDefaultBranch aborts on the default branch, passes elsewhere', () => {
  assert.throws(() => assertNotDefaultBranch('main', 'main'), /default branch \(main\)/);
  assert.doesNotThrow(() => assertNotDefaultBranch('chore/catalog-sync-2026-10-28', 'main'));
});
