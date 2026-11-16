#!/usr/bin/env node

// Capture the laptop-local proxy catalogs plus models.dev into a portable
// directory that provider-catalog-sync can consume without reaching the
// laptop or the network. Unavailable proxy ports are recorded in the
// manifest and intentionally omitted: the sync report marks those providers
// not-enumerable instead of treating a missing snapshot as an empty catalog.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { PROVIDER_SOURCES } from './provider-catalog-sync.mjs';

export const DEFAULT_MODELS_DEV_URL = 'https://models.dev/api.json';

function snapshotPorts() {
  return [...new Set(Object.values(PROVIDER_SOURCES)
    .filter((source) => source.kind === 'proxy')
    .map((source) => source.port))].sort((a, b) => a - b);
}

async function getJson(fetchImpl, url, timeoutMs) {
  const response = await fetchImpl(url, { signal: AbortSignal.timeout(timeoutMs) });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return response.json();
}

function removeIfPresent(file) {
  try {
    fs.unlinkSync(file);
  } catch (error) {
    if (error?.code !== 'ENOENT') throw error;
  }
}

export async function createSnapshot({
  outputDir,
  fetchImpl = fetch,
  modelsDevUrl = DEFAULT_MODELS_DEV_URL,
  now = new Date(),
}) {
  if (!outputDir) throw new Error('outputDir is required');
  fs.mkdirSync(outputDir, { recursive: true });
  const manifest = {
    format: 1,
    captured_at: now.toISOString(),
    proxy_ports: {},
    models_dev: { url: modelsDevUrl, file: 'models.dev.json', status: 'pending' },
  };

  for (const port of snapshotPorts()) {
    const url = `http://127.0.0.1:${port}/v1/models`;
    const file = path.join(outputDir, `${port}.json`);
    // Do not let a failed refresh silently fall back to a previous capture.
    removeIfPresent(file);
    try {
      const payload = await getJson(fetchImpl, url, 10000);
      if (!Array.isArray(payload?.data) || payload.data.some((model) => typeof model?.id !== 'string')) {
        throw new Error('response must contain data with string model ids');
      }
      const catalog = { data: payload.data.map(({ id }) => ({ id })) };
      fs.writeFileSync(file, `${JSON.stringify(catalog, null, 2)}\n`);
      manifest.proxy_ports[port] = { url, status: 'captured', model_count: catalog.data.length };
    } catch (error) {
      manifest.proxy_ports[port] = { url, status: 'unavailable', note: error?.message ?? String(error) };
    }
  }

  const modelsDevFile = path.join(outputDir, manifest.models_dev.file);
  removeIfPresent(modelsDevFile);
  try {
    const modelsDev = await getJson(fetchImpl, modelsDevUrl, 30000);
    if (modelsDev === null || typeof modelsDev !== 'object' || Array.isArray(modelsDev)) {
      throw new Error('models.dev response must be an object keyed by provider');
    }
    fs.writeFileSync(modelsDevFile, `${JSON.stringify(modelsDev, null, 2)}\n`);
    manifest.models_dev.status = 'captured';
    manifest.models_dev.provider_count = Object.keys(modelsDev).length;
  } catch (error) {
    manifest.models_dev.status = 'unavailable';
    manifest.models_dev.note = error?.message ?? String(error);
  }

  fs.writeFileSync(path.join(outputDir, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  if (manifest.models_dev.status !== 'captured') {
    throw new Error(`models.dev snapshot failed; see ${path.join(outputDir, 'manifest.json')}`);
  }
  return manifest;
}

export function parseArgs(argv) {
  const values = new Map();
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    if (flag !== '--output-dir' && flag !== '--models-dev-url') {
      throw new Error(`unrecognised argument: ${flag}`);
    }
    if (values.has(flag)) throw new Error(`${flag} was given more than once`);
    const value = argv[i + 1];
    if (!value || value.startsWith('--')) throw new Error(`${flag} requires a value`);
    values.set(flag, value);
    i += 1;
  }
  if (!values.has('--output-dir')) throw new Error('usage: snapshot-provider-catalogs --output-dir <directory> [--models-dev-url <url>]');
  return {
    outputDir: path.resolve(values.get('--output-dir')),
    modelsDevUrl: values.get('--models-dev-url') ?? DEFAULT_MODELS_DEV_URL,
  };
}

const isMain = (() => {
  try {
    return process.argv[1] && fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();

if (isMain) {
  try {
    const options = parseArgs(process.argv.slice(2));
    const manifest = await createSnapshot(options);
    console.log(`Provider catalog snapshot: ${options.outputDir}`);
    console.log(`Captured ${Object.values(manifest.proxy_ports).filter((port) => port.status === 'captured').length}/${Object.keys(manifest.proxy_ports).length} proxy ports; models.dev captured.`);
  } catch (error) {
    console.error(`[catalog-snapshot] ${error?.message ?? error}`);
    process.exitCode = 1;
  }
}
