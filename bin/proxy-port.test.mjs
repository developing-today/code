// Tests for the shared proxy port parser (#483).
// Run: node --test bin/proxy-port.test.mjs
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  parseProxyPortValue,
  parseProxyPortListValue,
  parseAddressPortValue,
  parseProxyMillisValue
} from './proxy-port.mjs';

const binDir = path.dirname(fileURLToPath(import.meta.url));

test('millisecond timeouts accept only decimal integers in the Node timer range', () => {
  for (const raw of [undefined, '']) assert.equal(parseProxyMillisValue('TIMEOUT', raw, 1200000), 1200000);
  for (const [raw, value] of [['1', 1], ['00100', 100], ['2147483647', 2147483647]]) {
    assert.equal(parseProxyMillisValue('TIMEOUT', raw, 1200000), value);
  }
  for (const raw of ['0', '-1', '+1', '1.5', '1e3', '0x10', ' 1', '1 ', 'abc', '2147483648', '999999999999999999999']) {
    assert.throws(() => parseProxyMillisValue('TIMEOUT', raw, 1200000), (err) =>
      err.message.includes('TIMEOUT') && err.message.includes(`"${raw}"`));
  }
});

test('unset and empty values fall back to the default', () => {
  assert.equal(parseProxyPortValue('X_PROXY_PORT', undefined, 18744), 18744);
  assert.equal(parseProxyPortValue('X_PROXY_PORT', '', 18744), 18744);
});

test('valid decimal ports are accepted exactly', () => {
  assert.equal(parseProxyPortValue('X_PROXY_PORT', '1', 18744), 1);
  assert.equal(parseProxyPortValue('X_PROXY_PORT', '18744', 1), 18744);
  assert.equal(parseProxyPortValue('X_PROXY_PORT', '65535', 1), 65535);
});

test('invalid values throw an error naming the env var and the value', () => {
  const invalid = [
    'abc',
    '0x4a38',
    '99999',
    '65536',
    '0',
    '-1',
    '18744.9',
    '18744abc',
    ' 18744',
    '+18744'
  ];
  for (const raw of invalid) {
    assert.throws(
      () => parseProxyPortValue('X_PROXY_PORT', raw, 18744),
      (err) => {
        assert.match(err.message, /X_PROXY_PORT/);
        assert.ok(
          err.message.includes(`"${raw}"`),
          `message should quote the value: ${err.message}`
        );
        assert.match(err.message, /integer 1-65535/);
        return true;
      },
      `expected ${JSON.stringify(raw)} to be rejected`
    );
  }
});

test('comma-separated proxy port lists preserve trimmed decimal ports and order', () => {
  assert.deepEqual(parseProxyPortListValue('RELAY_TARGET_PORTS', '18745,19745'), [
    18745,
    19745
  ]);
  assert.deepEqual(
    parseProxyPortListValue('RELAY_TARGET_PORTS', ' 18745 , 019745 , 18745 '),
    [18745, 19745, 18745]
  );
});

test('comma-separated proxy port lists reject invalid and empty segments', () => {
  const invalid = [
    ['18745,19745junk', '19745junk'],
    ['garbage', 'garbage'],
    ['18745,99999', '99999'],
    ['18745,19745.9', '19745.9'],
    ['1e4', '1e4'],
    ['0x4a39', '0x4a39'],
    ['18745,,19745', '18745,,19745'],
    ['18745,', '18745,'],
    [',18745', ',18745']
  ];
  for (const [raw, offending] of invalid) {
    assert.throws(
      () => parseProxyPortListValue('RELAY_TARGET_PORTS', raw),
      (err) => {
        assert.match(err.message, /RELAY_TARGET_PORTS/);
        assert.ok(err.message.includes(`"${offending}"`), err.message);
        return true;
      },
      `expected ${JSON.stringify(raw)} to be rejected`
    );
  }
  for (const raw of [undefined, '']) {
    assert.throws(
      () => parseProxyPortListValue('RELAY_TARGET_PORTS', raw),
      /RELAY_TARGET_PORTS is required/
    );
  }
});

test('host:port addresses are parsed strictly (#494)', () => {
  assert.equal(parseAddressPortValue('ANTIGRAVITY_LS_ADDRESS', '127.0.0.1:18745'), 18745);
  assert.equal(parseAddressPortValue('ANTIGRAVITY_LS_ADDRESS', 'localhost:1'), 1);
  assert.equal(parseAddressPortValue('ANTIGRAVITY_LS_ADDRESS', '[::1]:65535'), 65535);
  const invalid = [
    '127.0.0.1:abc',
    '127.0.0.1:',
    '127.0.0.1:0',
    '127.0.0.1:-5',
    '127.0.0.1:99999',
    '127.0.0.1:65536',
    '127.0.0.1:18745abc',
    '127.0.0.1:18745.9',
    '127.0.0.1: 18745'
  ];
  for (const raw of invalid) {
    assert.throws(
      () => parseAddressPortValue('ANTIGRAVITY_LS_ADDRESS', raw),
      (err) => {
        assert.match(err.message, /ANTIGRAVITY_LS_ADDRESS/);
        assert.ok(err.message.includes(`"${raw}"`), err.message);
        assert.match(err.message, /integer 1-65535/);
        return true;
      },
      `expected ${JSON.stringify(raw)} to be rejected`
    );
  }
});

const runRelay = (value) =>
  spawnSync(process.execPath, [path.join(binDir, 'proxy-relay.mjs')], {
    env: {
      ...process.env,
      RELAY_LISTEN_PORT: '18744',
      RELAY_TARGET_PORTS: value
    },
    encoding: 'utf8',
    timeout: 10000
  });


test('proxy-relay rejects every invalid target segment at startup', () => {
  const invalid = [
    ['garbage', 'garbage'],
    ['18745,19745junk', '19745junk'],
    ['18745,99999', '99999'],
    ['18745,19745.9', '19745.9'],
    ['1e4', '1e4'],
    ['0x4a39', '0x4a39'],
    ['18745,,19745', '18745,,19745'],
    ['18745,', '18745,'],
    [',18745', ',18745']
  ];
  for (const [raw, offending] of invalid) {
    const result = runRelay(raw);
    assert.equal(result.status, 1, `value ${raw}: ${result.stderr}`);
    assert.match(result.stderr, /RELAY_TARGET_PORTS/);
    assert.ok(result.stderr.includes(`"${offending}"`), result.stderr);
    assert.doesNotMatch(result.stdout, /listening/);
  }
});

test('proxy-relay starts with an ordered valid target list', async (t) => {
  const probe = net.createServer();
  await new Promise((resolve) => probe.listen(0, '127.0.0.1', resolve));
  const port = probe.address().port;
  await new Promise((resolve) => probe.close(resolve));

  const stateHome = fs.mkdtempSync(path.join(os.tmpdir(), 'proxy-relay-test-'));
  const child = spawn(process.execPath, [path.join(binDir, 'proxy-relay.mjs')], {
    env: {
      ...process.env,
      RELAY_LISTEN_HOST: '127.0.0.1',
      RELAY_LISTEN_PORT: String(port),
      RELAY_TARGET_PORTS: '18745,19745',
      RELAY_NAME: 'relay-valid-targets',
      XDG_STATE_HOME: stateHome
    },
    stdio: ['ignore', 'pipe', 'pipe']
  });
  t.after(async () => {
    child.kill();
    await new Promise((resolve) => {
      if (child.exitCode !== null) resolve();
      else child.once('exit', resolve);
    });
    fs.rmSync(stateHome, { recursive: true, force: true });
  });

  const listening = await new Promise((resolve, reject) => {
    let out = '';
    let err = '';
    const timer = setTimeout(
      () => reject(new Error(`no listening line: ${out} ${err}`)),
      10000
    );
    child.stdout.on('data', (chunk) => {
      out += chunk;
      if (out.includes('relay http://')) {
        clearTimeout(timer);
        resolve(out);
      }
    });
    child.stderr.on('data', (chunk) => { err += chunk; });
    child.on('exit', () => {
      clearTimeout(timer);
      reject(new Error(`proxy-relay exited early: ${out} ${err}`));
    });
  });
  assert.ok(listening.includes('18745|19745'), listening);

  const response = await fetch(`http://127.0.0.1:${port}/health`);
  assert.equal(response.status, 200);
  const health = await response.json();
  assert.equal(health.target, '100.99.113.103:18745|19745');
  assert.equal(health.preferredTarget, 18745);
  const endpoint = JSON.parse(
    fs.readFileSync(
      path.join(stateHome, 'relay-valid-targets', 'endpoint.json'),
      'utf8'
    )
  );
  assert.equal(endpoint.relayTo, '100.99.113.103:18745');
});

test('every proxy port parse site in the surviving proxies uses the shared helper', () => {
  // Per-file inventory, named, so a rework that moves a port from one proxy
  // to another has to update this list on purpose instead of drifting a bare
  // total. The retired doors (agy-proxy, antigravity-ls-proxy and
  // design-code-proxy — retired 2026-10-30/2026-11-03, their bins kept
  // as history until then — plus claude-code-proxy,
  // commandcode-proxy and codex-proxy) left this list with their bin programs in the
  // 2026-11-06 proxy-infra decommission, and muse-proxy — the last
  // provider proxy — left it the same day when muse-direct went
  // native in the fork (opencode#1825). The list is empty: no
  // provider proxy remains. proxy-supervise and proxy-relay still
  // use the shared helper but take no per-proxy port variable.
  const expected = {};
  for (const [file, names] of Object.entries(expected)) {
    const source = fs.readFileSync(path.join(binDir, file), 'utf8');
    assert.match(
      source,
      /import \{[^}]*\bparseProxyPort\b[^}]*\} from '\.\/proxy-port\.mjs';/,
      `${file} should import the shared helper`
    );
    assert.doesNotMatch(
      source,
      /parseInt\(process\.env\.[A-Z_]+PORT/,
      `${file} still parses a port with bare parseInt`
    );
    const parsed = [
      ...source.matchAll(/parseProxyPort\('([A-Z_]+)', \d+\)/g)
    ].map((m) => m[1]);
    assert.deepEqual(
      parsed.sort(),
      [...names].sort(),
      `${file} should parse ${names.join(', ')} through the helper`
    );
  }
});
