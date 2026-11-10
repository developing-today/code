import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import {
  parseArgs,
  UsageError,
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
  resolveConfigPath,
  defaultRuntimeConfigPath,
  deepMergeProviders,
  SYNC_OWNED_PROVIDERS,
} from './provider-catalog-sync.mjs';

const FIXTURE = `{
  // fleet config fixture
  "disabled_providers": ["groq"],
  "provider": {
    "proxy-a": {
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "http://127.0.0.1:18799/v1" },
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
      'fixture-proxy': { models: { 'model-a': {}, 'model-b': {} } },
    },
  }));
  fs.writeFileSync(path.join(sourceDir, '18799.json'), JSON.stringify(snapshot));
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
    const entry = report.providers['fixture-proxy'];
    assert.equal(entry.status, 'not-enumerable');
    assert.match(entry.note, /proxy :18799 enumeration failed:/);
    assert.equal(entry.live_count, undefined);
    assert.equal(entry.vanishings_basis, undefined);
  });
}

test('runSync preserves the valid proxy snapshot report', async () => {
  const { report } = await runSnapshot({ data: [{ id: 'model-a' }, { id: 'model-b' }] });
  const entry = report.providers['fixture-proxy'];
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

test('parseJsonc preserves commas before brackets inside strings (#3430)', () => {
  assert.equal(parseJsonc('{ "note": "x,}", "n": 1 }').note, 'x,}');
  assert.equal(parseJsonc('{ "note": "a,]", "n": 1 }').note, 'a,]');
  assert.equal(parseJsonc('{ "note": "a, }", "n": 1 }').note, 'a, }');
  assert.equal(parseJsonc('{ "list": ["x, ]", "y"] }').list[0], 'x, ]');
  const nested = parseJsonc(
    '{ "provider": { "p": { "models": { "m": { "variants": { "v": { "headers": "a,}" } } } } } } }',
  );
  assert.equal(nested.provider.p.models.m.variants.v.headers, 'a,}');
});

test('parseJsonc still removes real trailing commas and strips comments (#3430)', () => {
  assert.deepEqual(parseJsonc('{ "a": 1, }'), { a: 1 });
  assert.deepEqual(parseJsonc('// hi\n{ "a": [1, 2,], "b": { "c": 3, } }'), {
    a: [1, 2],
    b: { c: 3 },
  });
  assert.equal(parseJsonc('{ "url": "https://example.com//x" }').url, 'https://example.com//x');
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

test('makeEntry: a sibling with variants:null is not a clone source - name-only entry, no throw (#6907)', () => {
  const siblings = { 'gpt-1-luna': { name: 'GPT-1 Luna (codex)', variants: null } };
  const e = makeEntry('proxy-a', 'gpt-1-terra', siblings, null);
  assert.deepEqual(e, { name: 'GPT 1 Terra (codex)' });
  const withLimit = makeEntry('proxy-a', 'gpt-1-terra', siblings, { limit: { context: 2000, output: 200 } });
  assert.deepEqual(withLimit, { name: 'GPT 1 Terra (codex)', limit: { context: 2000, output: 200 } });
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

// ---------------------------------------------------------------------------
// Strict argv validation (issue #3449): invalid argv aborts as a usage error
// naming the flag, before any file read, fetch, or write; controls keep the
// old behaviour (dry-run exit 3 no write, --apply exit 0 writes).
// ---------------------------------------------------------------------------
const SCRIPT = new URL('./provider-catalog-sync.mjs', import.meta.url).pathname;

test('parseArgs rejects unknown and typo arguments naming them', () => {
  assert.throws(() => parseArgs(['--aply']), (e) => e instanceof UsageError && e.message.includes('--aply'));
  assert.throws(() => parseArgs(['--bogus-flag']), /--bogus-flag/);
  assert.throws(() => parseArgs(['positional']), /positional/);
});

test('parseArgs rejects flag-as-value and trailing value flags naming the flag', () => {
  assert.throws(() => parseArgs(['--config', '--apply']), (e) => e instanceof UsageError && e.message.includes('--config'));
  assert.throws(() => parseArgs(['--config']), /--config/);
  // '--models-dev --apply' was consumed twice (spec + apply) before #3449.
  assert.throws(() => parseArgs(['--models-dev', '--apply']), /--models-dev/);
  assert.throws(() => parseArgs(['--config', 'a.jsonc', '--config', 'b.jsonc']), /--config/);
});

test('parseArgs accepts the documented controls', () => {
  const dry = parseArgs([]);
  assert.deepEqual(dry.options, {});
  assert.equal(dry.flags.size, 0);
  const full = parseArgs(['--config', 'c.jsonc', '--models-dev', 'm.json', '--source-dir', 'src', '--repo', 'r', '--apply', '--open-pr', '--json']);
  assert.equal(full.options['--config'], 'c.jsonc');
  assert.equal(full.options['--models-dev'], 'm.json');
  assert.ok(full.flags.has('--apply') && full.flags.has('--open-pr') && full.flags.has('--json'));
});

function runCli(args, env = {}) {
  const clean = { ...process.env, ...env };
  // Scrub the ambient overlay variable unless a test passes it explicitly
  // (#4588: the explicit-pass case is the regression under test).
  if (!Object.hasOwn(env, 'OPENCODE_CONFIG_PATH')) delete clean.OPENCODE_CONFIG_PATH;
  return spawnSync(process.execPath, [SCRIPT, ...args], { encoding: 'utf8', env: clean });
}

test('CLI: invalid argv exits 2 naming the flag, before any read or write', () => {
  for (const [args, named] of [
    [['--aply'], '--aply'],
    [['--bogus-flag'], '--bogus-flag'],
    [['--config', '--apply'], '--config'],
    [['--config'], '--config'],
    [['--models-dev', '--apply'], '--models-dev'],
  ]) {
    const r = runCli(args);
    assert.equal(r.status, 2, `${args} -> ${r.stdout} ${r.stderr}`);
    assert.ok(r.stderr.includes(named), `${args} stderr names ${named}: ${r.stderr}`);
  }
});

function controlFixture() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'catalog-sync-argv-'));
  const configPath = path.join(dir, 'opencode.jsonc');
  const configText = `{
  "provider": {
    "fixture-proxy": {
      "npm": "@ai-sdk/openai-compatible",
      "models": {
        "acme-1": { "name": "Acme 1 (fixture-proxy)" }
      }
    }
  }
}
`;
  fs.writeFileSync(configPath, configText);
  const sourceDir = path.join(dir, 'src');
  fs.mkdirSync(sourceDir);
  fs.writeFileSync(path.join(sourceDir, '18799.json'), JSON.stringify({ data: [{ id: 'acme-1' }, { id: 'acme-2' }] }));
  const modelsDev = path.join(dir, 'models-dev.json');
  fs.writeFileSync(modelsDev, '{}');
  return { configPath, configText, sourceDir, modelsDev };
}

test('CLI control: dry run exits 3 without writing; --apply exits 0 and writes', () => {
  const { configPath, configText, sourceDir, modelsDev } = controlFixture();
  const base = ['--config', configPath, '--models-dev', modelsDev, '--source-dir', sourceDir];
  const dry = runCli(base);
  assert.equal(dry.status, 3, `dry: ${dry.stdout} ${dry.stderr}`);
  assert.equal(fs.readFileSync(configPath, 'utf8'), configText);
  const applied = runCli([...base, '--apply']);
  assert.equal(applied.status, 0, `apply: ${applied.stdout} ${applied.stderr}`);
  assert.ok(fs.readFileSync(configPath, 'utf8').includes('acme-2'));
});

// ---------------------------------------------------------------------------
// #4588: config resolution — OPENCODE_CONFIG_PATH names the runtime overlay
// since #4544 and must never become this tool's target
// ---------------------------------------------------------------------------

test('defaultRuntimeConfigPath: home-resolution contract probe matrix (#5762/#5763)', () => {
  // Oracle: the contract - unset and empty HOME are both a named
  // error; a set HOME yields exactly the historical path.
  assert.throws(() => defaultRuntimeConfigPath({}), /HOME is not set or is empty/);
  assert.throws(() => defaultRuntimeConfigPath({ HOME: '' }), /HOME is not set or is empty/);
  assert.equal(
    defaultRuntimeConfigPath({ HOME: '/home/someone' }),
    path.join('/home/someone', '.local/state/opencode/opencode-runtime.jsonc'),
  );
});

test('resolveConfigPath: --config beats env, catalog env beats default, OPENCODE_CONFIG_PATH ignored (#4588)', () => {
  const root = '/repo';
  const overlay = '/state/opencode-runtime.jsonc';
  const dflt = path.join(root, '.opencode', 'opencode.jsonc');
  // Explicit flag wins over every env var.
  assert.equal(
    resolveConfigPath({ options: { '--config': '/flag.jsonc' }, env: { OPENCODE_CONFIG_PATH: overlay, OPENCODE_CATALOG_SYNC_CONFIG: '/env.jsonc' }, root }),
    '/flag.jsonc',
  );
  // Catalog-sync-specific env is the only env override.
  assert.equal(
    resolveConfigPath({ options: {}, env: { OPENCODE_CONFIG_PATH: overlay, OPENCODE_CATALOG_SYNC_CONFIG: '/env.jsonc' }, root }),
    '/env.jsonc',
  );
  // OPENCODE_CONFIG_PATH alone changes nothing: tracked default.
  assert.equal(resolveConfigPath({ options: {}, env: { OPENCODE_CONFIG_PATH: overlay }, root }), dflt);
  assert.equal(resolveConfigPath({ options: {}, env: {}, root }), dflt);
});

test('CLI: OPENCODE_CONFIG_PATH pointing at the overlay does not redirect the target (#4588)', () => {
  const { configPath, sourceDir, modelsDev } = controlFixture();
  const dir = path.dirname(configPath);
  const overlayPath = path.join(dir, 'opencode-runtime.jsonc');
  fs.writeFileSync(overlayPath, JSON.stringify({ provider: { openai: { models: { 'g-1': {} } } } }));
  const r = runCli(
    ['--config', configPath, '--runtime-config', overlayPath, '--models-dev', modelsDev, '--source-dir', sourceDir, '--json'],
    { OPENCODE_CONFIG_PATH: overlayPath },
  );
  // The tracked file carries fixture-proxy/acme-1 and the snapshot adds acme-2:
  // a run against the tracked default reports the validated addition (exit 3).
  // Against the overlay (the bug) it would report config=overlay and exit 0.
  assert.equal(r.status, 3, `stdout: ${r.stdout} stderr: ${r.stderr}`);
  const report = JSON.parse(r.stdout);
  assert.equal(report.config, configPath);
  assert.equal(report.runtime_overlay, overlayPath);
  assert.ok(report.providers['fixture-proxy'], `providers: ${Object.keys(report.providers)}`);
});

// ---------------------------------------------------------------------------
// #4544: runtime overlay (sync-models-owned catalogs outside the tracked
// configured default)
// ---------------------------------------------------------------------------

test('SYNC_OWNED_PROVIDERS names exactly the four sync-models-owned providers (#4544)', () => {
  assert.deepEqual(
    [...SYNC_OWNED_PROVIDERS].sort(),
    ['_antigravity', 'cloudflare-gateway', 'google', 'openai'],
  );
});

test('deepMergeProviders merges catalogs per model, overlay wins on conflict (#4544)', () => {
  const base = { google: { models: { a: { name: 'A' }, b: { name: 'B', limit: { context: 1 } } } }, x: { models: { m: {} } } };
  const over = { google: { models: { b: { name: 'B2' }, c: { name: 'C' } } } };
  const merged = deepMergeProviders(base, over);
  assert.deepEqual(Object.keys(merged.google.models).sort(), ['a', 'b', 'c']);
  assert.equal(merged.google.models.b.name, 'B2');
  assert.equal(merged.google.models.a.name, 'A');
  assert.deepEqual(Object.keys(merged.x.models), ['m']);
});

test('deepMergeProviders defines external keys at depth >= 2 instead of assigning (#4957)', () => {
  // Design #2953 corollary 4 probe matrix, one level deeper than the merge
  // root: the overlay is an external file, so every key it spells must be
  // an own data property rather than a [[Set]] write that can reach a
  // prototype. JSON.parse (not object-literal syntax) creates the own
  // __proto__ key exactly as the config parser does.
  const base = { p: { models: { m: { limit: { context: 1 } } } } };
  const over = JSON.parse(
    '{"p":{"models":{"m":{"limit":{' +
      '"__proto__":{"polluted":true},"constructor":"c","toString":"t","ordinary":42}}}}}',
  );
  const merged = deepMergeProviders(base, over);
  const leaf = merged.p.models.m.limit;
  assert.equal(Object.getPrototypeOf(leaf), Object.prototype);
  assert.ok(Object.prototype.hasOwnProperty.call(leaf, '__proto__'));
  assert.deepEqual(leaf.__proto__, { polluted: true });
  assert.equal(leaf.constructor, 'c');
  assert.equal(leaf.toString, 't');
  assert.equal(leaf.ordinary, 42);
  assert.equal(leaf.context, 1);
  assert.equal(Object.prototype.polluted, undefined);
  // JSON round-trip: the own key survives serialization and the prototype
  // stays put.
  const roundTrip = JSON.parse(JSON.stringify(merged));
  assert.equal(Object.getPrototypeOf(roundTrip.p.models.m.limit), Object.prototype);
  assert.ok(Object.prototype.hasOwnProperty.call(roundTrip.p.models.m.limit, '__proto__'));
  assert.deepEqual(roundTrip.p.models.m.limit.__proto__, { polluted: true });
});

test('deepMergeProviders defines a string __proto__ at depth 3 without moving prototypes (#4957)', () => {
  const base = { p: { models: { m: { variants: { low: { options: {} } } } } } };
  const over = JSON.parse('{"p":{"models":{"m":{"variants":{"low":{"options":{"__proto__":"text"}}}}}}}');
  const merged = deepMergeProviders(base, over);
  const options = merged.p.models.m.variants.low.options;
  assert.equal(Object.getPrototypeOf(options), Object.prototype);
  assert.ok(Object.prototype.hasOwnProperty.call(options, '__proto__'));
  assert.equal(options.__proto__, 'text');
  assert.equal(Object.getPrototypeOf(merged.p.models.m.variants.low), Object.prototype);
});

test('runSync diffs the merged effective config when a runtime overlay exists (#4544)', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'catalog-sync-overlay-'));
  const configPath = path.join(dir, 'opencode.jsonc');
  // Tracked default post-split: no google block at all (it was models-only).
  fs.writeFileSync(configPath, '{ "provider": { "deepseek": { "models": { "ds-1": {} } } } }');
  const overlayPath = path.join(dir, 'opencode-runtime.jsonc');
  fs.writeFileSync(
    overlayPath,
    JSON.stringify({ provider: { google: { models: { 'gem-a': {}, 'gem-b': {} } } } }),
  );
  const modelsDev = path.join(dir, 'models-dev.json');
  fs.writeFileSync(modelsDev, '{}');
  const { report } = await runSync({
    configPath,
    modelsDevSpec: modelsDev,
    sourceDir: null,
    runtimeConfigPath: overlayPath,
  });
  assert.equal(report.runtime_overlay, overlayPath);
  assert.equal(report.providers.google.config_count, 2);
  // Without the overlay the same tracked file has no google provider at all.
  const bare = await runSync({
    configPath,
    modelsDevSpec: modelsDev,
    sourceDir: null,
    runtimeConfigPath: path.join(dir, 'absent.jsonc'),
  });
  assert.equal(bare.report.runtime_overlay, null);
  assert.equal(bare.report.providers.google, undefined);
});
