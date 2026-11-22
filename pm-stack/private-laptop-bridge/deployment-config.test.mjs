import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

const read = name => readFile(new URL(name, import.meta.url), 'utf8');
function parseJsonc(source) {
  // These repository configs use line comments only and do not contain URLs
  // in JSON strings with //; keep this helper deliberately limited.
  return JSON.parse(source.split('\n').filter(line => !line.trimStart().startsWith('//')).join('\n'));
}

test('Wrangler local and staging definitions preserve the same DO class and SQLite migration', async () => {
  const local = parseJsonc(await read('./wrangler.jsonc'));
  const staging = parseJsonc(await read('./wrangler.staging.jsonc'));
  for (const config of [local, staging]) {
    assert.equal(config.name, 'private-laptop-bridge-staging');
    assert.equal(config.compatibility_date, '2026-07-30');
    assert.equal(config.workers_dev, false);
    assert.equal(config.preview_urls, false);
    assert.deepEqual(config.durable_objects.bindings, [{name:'BRIDGE',class_name:'PrivateLaptopBridge'}]);
    assert.deepEqual(config.migrations, [{tag:'v1',new_sqlite_classes:['PrivateLaptopBridge']}]);
  }
  assert.equal(local.main, 'worker.mjs');
  assert.equal(staging.main, 'gateway.mjs');
});

test('cf release config matches the reviewed private bridge resources and remains disabled', async () => {
  const config = await read('./cloudflare.config.ts');
  assert.match(config, /name:\s*'private-laptop-bridge-staging'/);
  assert.match(config, /mode\s*===\s*'bridge'/);
  assert.match(config, /entrypoint:\s*bridge/);
  assert.match(config, /PrivateLaptopBridge:\s*exports\.durableObject\(\{storage:\s*'sqlite'\}\)/);
  assert.match(config, /BRIDGE:\s*bindings\.durableObject\(\{worker:\s*'private-laptop-bridge-staging',\s*exportName:\s*'PrivateLaptopBridge'\}\)/);
  assert.match(config, /ENABLED:\s*bindings\.text\('false'\)/);
  assert.match(config, /GATEWAY_HOST:\s*bindings\.text\('private-bridge-staging\.invalid'\)/);
  assert.match(config, /BRIDGE_REGISTRY:\s*bindings\.text\(''\)/);
});

test('pinned cf release and Wrangler producer satisfy the documented Build Output v1 pair', async () => {
  const pkg = JSON.parse(await read('./package.json'));
  const lock = JSON.parse(await read('./package-lock.json'));
  assert.equal(pkg.devDependencies.cf, '1.0.0-rc.1');
  assert.equal(lock.packages['node_modules/cf'].version, pkg.devDependencies.cf);
  assert.equal(pkg.devDependencies.wrangler, '4.166.0');
  assert.equal(lock.packages['node_modules/wrangler'].version, pkg.devDependencies.wrangler);
  assert.equal(lock.packages['node_modules/@cloudflare/build-output-utils'].version, '1.0.0-rc.1');
});
