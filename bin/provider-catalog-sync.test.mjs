import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
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

test('CLI: entry point runs main via direct path, file symlink, and directory symlink (#2235)', () => {
  // Regression: the guard compared the raw invoked path with the resolved
  // module URL, so symlinked invocations silently skipped main (exit 0,
  // no output). With a nonexistent --config, a main that RAN fails loudly
  // (exit 2, '[catalog-sync] fatal:'), which is exactly what distinguishes
  // the two behaviours without any network or repo state.
  const binDir = path.dirname(fileURLToPath(import.meta.url));
  const cli = path.join(binDir, 'provider-catalog-sync.mjs');
  const linkDir = fs.mkdtempSync(path.join(os.tmpdir(), 'catalog-sync-link-'));
  const fileLink = path.join(linkDir, 'catalog-sync-link.mjs');
  fs.symlinkSync(cli, fileLink);
  const dirLink = path.join(linkDir, 'bin-link');
  fs.symlinkSync(binDir, dirLink, 'dir');

  for (const invoked of [cli, fileLink, path.join(dirLink, 'provider-catalog-sync.mjs')]) {
    const res = spawnSync(process.execPath, [invoked, '--config', path.join(linkDir, 'missing.jsonc')], {
      encoding: 'utf8'
    });
    assert.equal(res.status, 2, `${invoked}: ${res.stdout} ${res.stderr}`);
    assert.match(res.stderr, /\[catalog-sync\] fatal:/, invoked);
  }
});

test('importing provider-catalog-sync.mjs as a library does not run main (#2235)', () => {
  const cli = path.join(path.dirname(fileURLToPath(import.meta.url)), 'provider-catalog-sync.mjs');
  const res = spawnSync(
    process.execPath,
    ['--input-type=module', '-e', `import ${JSON.stringify(pathToFileURL(cli).href)};`],
    { encoding: 'utf8', env: { ...process.env, OPENCODE_CONFIG_PATH: '/nonexistent.jsonc' } }
  );
  assert.equal(res.status, 0, res.stderr);
  assert.equal(res.stdout, '');
  assert.equal(res.stderr, '');
});

test('assertNotDefaultBranch aborts on the default branch, passes elsewhere', () => {
  assert.throws(() => assertNotDefaultBranch('main', 'main'), /default branch \(main\)/);
  assert.doesNotThrow(() => assertNotDefaultBranch('chore/catalog-sync-2026-10-28', 'main'));
});
