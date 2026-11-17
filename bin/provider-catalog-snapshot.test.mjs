import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createSnapshot, parseArgs } from './snapshot-provider-catalogs.mjs';
import { PROVIDER_SOURCES, runSync } from './provider-catalog-sync.mjs';

const PROXY_PORTS = [...new Set(Object.values(PROVIDER_SOURCES)
  .filter((source) => source.kind === 'proxy')
  .map((source) => source.port))].sort((a, b) => a - b);
const [FIRST_PORT] = PROXY_PORTS;

function response(status, body) {
  return { ok: status >= 200 && status < 300, status, json: async () => body };
}

test('snapshot enumerates every proxy source port the source table declares', () => {
  // The table itself is the oracle: the snapshot must reach exactly the
  // proxy ports it declares, no more and no fewer, and dedupe shared doors.
  assert.ok(PROXY_PORTS.length >= 1);
  assert.equal(new Set(PROXY_PORTS).size, PROXY_PORTS.length);
});

test('snapshot captures unique proxy catalogs, models.dev, and source status', async () => {
  const outputDir = fs.mkdtempSync(path.join(os.tmpdir(), 'catalog-snapshot-'));
  const calls = [];
  const manifest = await createSnapshot({
    outputDir,
    now: new Date('2026-11-03T01:00:00.000Z'),
    fetchImpl: async (url) => {
      calls.push(url);
      if (url.includes('models.dev')) return response(200, { openai: { models: { gpt: {} } } });
      return response(200, { data: [{ id: `model-${new URL(url).port}`, extra: 'not copied' }] });
    },
  });

  assert.equal(
    new Set(calls.filter((url) => url.startsWith('http://'))).size,
    PROXY_PORTS.length,
  );
  assert.equal(manifest.captured_at, '2026-11-03T01:00:00.000Z');
  for (const port of PROXY_PORTS) {
    assert.equal(manifest.proxy_ports[port].status, 'captured');
    assert.deepEqual(JSON.parse(fs.readFileSync(path.join(outputDir, `${port}.json`), 'utf8')),
      { data: [{ id: `model-${port}` }] });
  }
  assert.equal(manifest.models_dev.status, 'captured');
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(outputDir, 'models.dev.json'), 'utf8')),
    { openai: { models: { gpt: {} } } });
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(outputDir, 'manifest.json'), 'utf8')), manifest);
});

test('an unavailable proxy port is recorded and its stale capture is removed', async () => {
  const outputDir = fs.mkdtempSync(path.join(os.tmpdir(), 'catalog-snapshot-proxy-'));
  fs.writeFileSync(path.join(outputDir, `${FIRST_PORT}.json`), '{"data":[{"id":"stale"}]}');
  const manifest = await createSnapshot({
    outputDir,
    fetchImpl: async (url) => {
      if (url.includes('models.dev')) return response(200, {});
      return response(503, {});
    },
  });
  assert.equal(manifest.proxy_ports[FIRST_PORT].status, 'unavailable');
  assert.match(manifest.proxy_ports[FIRST_PORT].note, /HTTP 503/);
  // A failed refresh must not leave the previous capture behind to be
  // mistaken for a fresh one.
  assert.equal(fs.existsSync(path.join(outputDir, `${FIRST_PORT}.json`)), false);
});

test('snapshot records a failed models.dev fetch and fails for offline use', async () => {
  const outputDir = fs.mkdtempSync(path.join(os.tmpdir(), 'catalog-snapshot-error-'));
  for (const port of PROXY_PORTS) {
    fs.writeFileSync(path.join(outputDir, `${port}.json`), '{"data":[{"id":"stale"}]}');
  }
  fs.writeFileSync(path.join(outputDir, 'models.dev.json'), '{"stale":true}');
  await assert.rejects(
    createSnapshot({ outputDir, fetchImpl: async () => response(502, {}) }),
    /models.dev snapshot failed/,
  );
  const manifest = JSON.parse(fs.readFileSync(path.join(outputDir, 'manifest.json'), 'utf8'));
  assert.equal(manifest.models_dev.status, 'unavailable');
  assert.match(manifest.models_dev.note, /HTTP 502/);
  for (const port of PROXY_PORTS) {
    assert.equal(fs.existsSync(path.join(outputDir, `${port}.json`)), false);
  }
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
  const source = Object.entries(PROVIDER_SOURCES).find(([, src]) => src.kind === 'proxy');
  assert.ok(source, 'the source table declares at least one proxy source');
  const [providerId, src] = source;

  const outputDir = fs.mkdtempSync(path.join(os.tmpdir(), 'catalog-snapshot-offline-'));
  const configPath = path.join(outputDir, 'opencode.jsonc');
  fs.writeFileSync(configPath, JSON.stringify({ provider: { [providerId]: { models: { existing: {} } } } }));

  await createSnapshot({
    outputDir,
    fetchImpl: async (url) => {
      if (url.includes('models.dev')) return response(200, {});
      const ids = new URL(url).port === String(src.port) ? ['existing', 'added'] : [];
      return response(200, { data: ids.map((id) => ({ id })) });
    },
  });

  const { report } = await runSync({
    configPath,
    sourceDir: outputDir,
    modelsDevSpec: path.join(outputDir, 'models.dev.json'),
    runtimeConfigPath: null,
  });
  assert.deepEqual(report.providers[providerId].additions_validated, ['added']);
  assert.deepEqual(report.providers[providerId].additions_unverified, []);
});
