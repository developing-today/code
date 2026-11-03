import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createSnapshot, parseArgs } from './snapshot-provider-catalogs.mjs';
import { PROVIDER_SOURCES, runSync } from './provider-catalog-sync.mjs';

function response(status, body) {
  return { ok: status >= 200 && status < 300, status, json: async () => body };
}

test('snapshot captures unique proxy catalogs, models.dev, and source status', async () => {
  const outputDir = fs.mkdtempSync(path.join(os.tmpdir(), 'catalog-snapshot-'));
  const calls = [];
  const manifest = await createSnapshot({
    outputDir,
    now: new Date('2026-11-03T01:00:00.000Z'),
    fetchImpl: async (url) => {
      calls.push(url);
      if (url.includes('models.dev')) return response(200, { openai: { models: { gpt: {} } } });
      if (url.includes(':18748/')) return response(503, {});
      return response(200, { data: [{ id: `model-${new URL(url).port}`, extra: 'not copied' }] });
    },
  });

  assert.equal(
    new Set(calls.filter((url) => url.startsWith('http://'))).size,
    new Set(Object.values(PROVIDER_SOURCES)
      .filter((source) => source.kind === 'proxy')
      .map((source) => source.port)).size,
  );
  assert.equal(manifest.captured_at, '2026-11-03T01:00:00.000Z');
  assert.equal(manifest.proxy_ports[18748].status, 'unavailable');
  assert.equal(manifest.proxy_ports[18744].status, 'captured');
  assert.equal(manifest.models_dev.status, 'captured');
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(outputDir, '18744.json'), 'utf8')),
    { data: [{ id: 'model-18744' }] });
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(outputDir, 'models.dev.json'), 'utf8')),
    { openai: { models: { gpt: {} } } });
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(outputDir, 'manifest.json'), 'utf8')), manifest);
  assert.equal(fs.existsSync(path.join(outputDir, '18748.json')), false);
});

test('snapshot records a failed models.dev fetch and fails for offline use', async () => {
  const outputDir = fs.mkdtempSync(path.join(os.tmpdir(), 'catalog-snapshot-error-'));
  fs.writeFileSync(path.join(outputDir, '18743.json'), '{"data":[{"id":"stale"}]}');
  fs.writeFileSync(path.join(outputDir, 'models.dev.json'), '{"stale":true}');
  await assert.rejects(
    createSnapshot({ outputDir, fetchImpl: async () => response(502, {}) }),
    /models.dev snapshot failed/,
  );
  const manifest = JSON.parse(fs.readFileSync(path.join(outputDir, 'manifest.json'), 'utf8'));
  assert.equal(manifest.models_dev.status, 'unavailable');
  assert.match(manifest.models_dev.note, /HTTP 502/);
  assert.equal(fs.existsSync(path.join(outputDir, '18743.json')), false);
  assert.equal(fs.existsSync(path.join(outputDir, 'models.dev.json')), false);
});

test('snapshot CLI arguments require an output directory and reject duplicates', () => {
  assert.throws(() => parseArgs([]), /--output-dir/);
  assert.throws(() => parseArgs(['--output-dir', 'a', '--output-dir', 'b']), /more than once/);
  assert.deepEqual(parseArgs(['--output-dir', 'snapshot']), {
    outputDir: path.resolve('snapshot'),
    modelsDevUrl: 'https://models.dev/api.json',
  });
});

test('captured files feed the offline provider-catalog diff', async () => {
  const outputDir = fs.mkdtempSync(path.join(os.tmpdir(), 'catalog-snapshot-offline-'));
  const configPath = path.join(outputDir, 'opencode.jsonc');
  fs.writeFileSync(configPath, JSON.stringify({ provider: { commandcode: { models: { existing: {} } } } }));

  await createSnapshot({
    outputDir,
    fetchImpl: async (url) => {
      if (url.includes('models.dev')) {
        return response(200, { commandcode: { models: { existing: {}, added: {} } } });
      }
      const ids = new URL(url).port === '18748' ? ['existing', 'added'] : [];
      return response(200, { data: ids.map((id) => ({ id })) });
    },
  });

  const { report } = await runSync({
    configPath,
    sourceDir: outputDir,
    modelsDevSpec: path.join(outputDir, 'models.dev.json'),
    runtimeConfigPath: null,
  });
  assert.deepEqual(report.providers.commandcode.additions_validated, ['added']);
  assert.deepEqual(report.providers.commandcode.additions_unverified, []);
});
