import test from 'node:test';
import assert from 'node:assert/strict';
import {readdir, readFile} from 'node:fs/promises';

const read = name => readFile(new URL(name, import.meta.url), 'utf8');

test('cf declarative bridge config is the sole lifecycle source and remains disabled', async () => {
  const config = await read('./cloudflare.config.ts');
  assert.match(config, /name:\s*'private-laptop-bridge-staging'/);
  assert.match(config, /mode\s*===\s*'bridge'/);
  assert.match(config, /entrypoint:\s*bridge/);
  assert.match(config, /PrivateLaptopBridge:\s*exports\.durableObject\(\{storage:\s*'sqlite'\}\)/);
  assert.match(config, /BRIDGE:\s*bindings\.durableObject\(\{worker:\s*'private-laptop-bridge-staging',\s*exportName:\s*'PrivateLaptopBridge'\}\)/);
  assert.match(config, /ENABLED:\s*bindings\.text\('false'\)/);
  assert.match(config, /GATEWAY_HOST:\s*bindings\.text\('private-bridge-staging\.invalid'\)/);
  assert.match(config, /BRIDGE_REGISTRY:\s*bindings\.text\(''\)/);
  assert.doesNotMatch(config, /migrations|new_sqlite_classes/);

  const files = await readdir(new URL('.', import.meta.url));
  assert.equal(files.some(name => /^wrangler(?:\.staging|\.smoke)?\.jsonc$/.test(name)), false);
});

test('pinned cf producer and Build Output consumer are the documented v1 pair', async () => {
  const pkg = JSON.parse(await read('./package.json'));
  const lock = JSON.parse(await read('./package-lock.json'));
  assert.equal(pkg.devDependencies.cf, '1.0.0-rc.1');
  assert.equal(lock.packages['node_modules/cf'].version, pkg.devDependencies.cf);
  assert.equal(pkg.devDependencies.wrangler, '4.166.0');
  assert.equal(lock.packages['node_modules/wrangler'].version, pkg.devDependencies.wrangler);
  assert.equal(lock.packages['node_modules/@cloudflare/build-output-utils'].version, '1.0.0-rc.1');
});
