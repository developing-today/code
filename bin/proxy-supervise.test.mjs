// Tests for proxy-supervise.mjs — hash-keyed A/B supervision.
// Run: node --test bin/proxy-supervise.test.mjs
//
// The integration tests run a fake "proxy" (a tiny node HTTP server
// answering /health like the fleet proxies) under a real supervisor
// config in a temp dir, and drive the reconcile state machine:
// start -> no-op -> hash change -> A/B roll; swap strategy; a broken
// new build keeping the old instance; adoption of a foreign instance.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  computeHash,
  expandPath,
  loadConfig,
  otherPort,
  pidAlive,
  reconcile,
  resolveEnv,
} from './proxy-supervise.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const FAKE = (marker) => `
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
const port = Number(process.env.FAKE_PROXY_PORT);
// Mimic the fleet proxies: publish a minimal endpoint.json on listen
// and UNLINK it on graceful exit (codex-proxy.mjs does exactly this —
// it is why the supervisor must rewrite the record after a roll).
const ep = process.env.FAKE_ENDPOINT;
if (ep) {
  fs.mkdirSync(path.dirname(ep), { recursive: true });
  fs.writeFileSync(ep, JSON.stringify({ host: '127.0.0.1', port, pid: process.pid, url: 'http://127.0.0.1:' + port }));
  process.on('SIGTERM', () => { try { fs.unlinkSync(ep); } catch {} process.exit(0); });
}
http.createServer((req, res) => {
  if (req.url === '/health') { res.writeHead(200, {'content-type':'application/json'}); res.end(JSON.stringify({status:'ok', marker:${JSON.stringify(marker)}})); }
  else { res.writeHead(404); res.end(); }
}).listen(port, '127.0.0.1');
`;

const getHealth = (port) =>
  new Promise((resolve) => {
    const req = http.get({ host: '127.0.0.1', port, path: '/health', timeout: 1500 }, (res) => {
      let b = '';
      res.on('data', (c) => (b += c));
      res.on('end', () => resolve({ status: res.statusCode, body: b }));
    });
    req.on('error', () => resolve({ status: 0, body: '' }));
    req.on('timeout', () => { req.destroy(); resolve({ status: 0, body: '' }); });
  });

const spawnedPids = new Set();
const track = (pid) => { if (pid) spawnedPids.add(pid); };
test.after(() => {
  for (const pid of spawnedPids) { try { process.kill(pid, 'SIGKILL'); } catch { /* gone */ } }
});

let portBase = 31400;
const makeWorld = async (t, { strategy = 'ab', script = FAKE('v1') } = {}) => {
  portBase += 10;
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'proxy-supervise-test-'));
  const stateDir = path.join(root, 'state');
  const endpointDir = path.join(root, 'endpoints');
  const scriptPath = path.join(root, 'fake-proxy.mjs');
  fs.writeFileSync(scriptPath, script);
  const canonicalPort = portBase;
  const configPath = path.join(root, 'config.json');
  fs.writeFileSync(configPath, JSON.stringify({
    stateDir,
    endpointDir,
    drainSeconds: 1,
    healthTimeoutSeconds: 12,
    proxies: [{
      name: 'fake-proxy',
      canonicalPort,
      altPortOffset: 1000,
      strategy,
      command: [process.execPath, scriptPath],
      portEnv: 'FAKE_PROXY_PORT',
      hashFiles: [scriptPath],
      env: { FAKE_ENDPOINT: path.join(endpointDir, 'fake-proxy', 'endpoint.json') },
    }],
  }));
  const cfg = loadConfig(configPath);
  const proxy = cfg.proxies[0];
  const quiet = { log: () => {} };
  const run = async () => {
    const r = await reconcile(cfg, proxy, quiet);
    const st = JSON.parse(fs.readFileSync(path.join(stateDir, 'fake-proxy.json'), 'utf8'));
    track(st?.current?.pid);
    return r;
  };
  t.after(() => {
    try {
      const st = JSON.parse(fs.readFileSync(path.join(stateDir, 'fake-proxy.json'), 'utf8'));
      if (st?.current?.pid) track(st.current.pid);
    } catch { /* no state */ }
  });
  return { root, stateDir, endpointDir, scriptPath, canonicalPort, altPort: canonicalPort + 1000, cfg, proxy, run };
};

test('expandPath expands $VAR, ${VAR} and ~', () => {
  assert.equal(expandPath('$FOO/bar', { FOO: '/x' }), '/x/bar');
  assert.equal(expandPath('${FOO}bar', { FOO: '/x' }), '/xbar');
  assert.equal(expandPath('~/a', {}), path.join(os.homedir(), 'a'));
  assert.equal(expandPath('/plain', {}), '/plain');
});

test('otherPort flips between canonical and alternate', () => {
  const p = { canonicalPort: 18744, altPortOffset: 1000 };
  assert.equal(otherPort(p, 18744), 19744);
  assert.equal(otherPort(p, 19744), 18744);
});

test('computeHash is stable, and changes with content, env, backend port', async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'proxy-supervise-hash-'));
  const f = path.join(root, 'a.mjs');
  fs.writeFileSync(f, 'one');
  const proxy = { command: [process.execPath, f], hashFiles: [f], hashEnv: ['K'], backends: [{ name: 'be', env: 'BE_PORT' }] };
  const env = { K: 'v' };
  const h1 = computeHash(proxy, env, { be: 18744 });
  assert.equal(h1, computeHash(proxy, env, { be: 18744 }));
  fs.writeFileSync(f, 'two');
  assert.notEqual(h1, computeHash(proxy, env, { be: 18744 }));
  fs.writeFileSync(f, 'one');
  assert.notEqual(h1, computeHash(proxy, { K: 'other' }, { be: 18744 }));
  assert.notEqual(h1, computeHash(proxy, env, { be: 19744 }));
});

test('resolveEnv injects backend ports', () => {
  const proxy = { env: { A: '1' }, backends: [{ name: 'codex-proxy', env: 'CODEX_PROXY_PORT' }] };
  const env = resolveEnv(proxy, { BASE: 'x' }, { 'codex-proxy': 19744 });
  assert.equal(env.CODEX_PROXY_PORT, '19744');
  assert.equal(env.A, '1');
  assert.equal(env.BASE, 'x');
});

// ---------- loadConfig validation (#1952) ----------

const loadBad = (config) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'proxy-supervise-badcfg-'));
  const configPath = path.join(root, 'config.json');
  fs.writeFileSync(configPath, JSON.stringify(config));
  let err = null;
  try {
    loadConfig(configPath);
  } catch (e) {
    err = e;
  }
  assert.ok(err, `expected loadConfig to reject ${JSON.stringify(config)}`);
  assert.ok(!(err instanceof TypeError), `error must not be an opaque TypeError: ${err.message}`);
  assert.ok(err.message.includes(configPath), `error must name the config file: ${err.message}`);
  return err.message;
};

const validProxy = { name: 'p1', command: ['/bin/true'], canonicalPort: 18742, portEnv: 'P1_PORT' };

test('loadConfig accepts an omitted proxies list and a valid minimal proxy', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'proxy-supervise-okcfg-'));
  const emptyPath = path.join(root, 'empty.json');
  fs.writeFileSync(emptyPath, JSON.stringify({}));
  assert.deepEqual(loadConfig(emptyPath).proxies, []);
  const okPath = path.join(root, 'ok.json');
  fs.writeFileSync(okPath, JSON.stringify({ proxies: [validProxy] }));
  const cfg = loadConfig(okPath);
  assert.equal(cfg.proxies[0].name, 'p1');
  assert.equal(cfg.drainSeconds, 5);
  assert.equal(cfg.healthTimeoutSeconds, 45);
  assert.equal(cfg.lockTimeoutSeconds, 900);
});

test('loadConfig rejects malformed top-level and entry shapes, naming the field', () => {
  assert.match(loadBad([]), /top-level value must be an object/);
  assert.match(loadBad({ proxies: {} }), /field 'proxies' must be an array/);
  assert.match(loadBad({ proxies: ['nope'] }), /proxies\[0\]: entry must be an object/);
  assert.match(loadBad({ proxies: [{ name: 'p1', canonicalPort: 18742, portEnv: 'P' }] }), /proxies\[0\] \('p1'\): field 'command' is required/);
  assert.match(loadBad({ proxies: [{ ...validProxy, command: 'run' }] }), /field 'command' must be an array/);
  assert.match(loadBad({ proxies: [{ ...validProxy, command: [] }] }), /field 'command' must be a non-empty array/);
  assert.match(loadBad({ proxies: [{ ...validProxy, command: ['/bin/true', 7] }] }), /field 'command\[1\]' must be a non-empty string/);
  assert.match(loadBad({ proxies: [{ command: ['/bin/true'], canonicalPort: 18742, portEnv: 'P' }] }), /field 'name' is required/);
  assert.match(loadBad({ proxies: [validProxy, validProxy] }), /field 'name' duplicates/);
  assert.match(loadBad({ proxies: [{ name: 'p1', command: ['/bin/true'], canonicalPort: 18742 }] }), /field 'portEnv' is required/);
});

test('loadConfig rejects bad ports and altPortOffset values', () => {
  assert.match(loadBad({ proxies: [{ ...validProxy, canonicalPort: 'abc' }] }), /field 'canonicalPort' must be an integer in 1\.\.=65535/);
  assert.match(loadBad({ proxies: [{ ...validProxy, canonicalPort: 0 }] }), /field 'canonicalPort'/);
  assert.match(loadBad({ proxies: [{ ...validProxy, canonicalPort: 65536 }] }), /field 'canonicalPort'/);
  assert.match(loadBad({ proxies: [{ ...validProxy, canonicalPort: 18742.5 }] }), /field 'canonicalPort'/);
  assert.match(loadBad({ proxies: [{ ...validProxy, altPortOffset: 0 }] }), /field 'altPortOffset' must be a positive integer/);
  assert.match(loadBad({ proxies: [{ ...validProxy, altPortOffset: -5 }] }), /field 'altPortOffset'/);
  assert.match(loadBad({ proxies: [{ ...validProxy, canonicalPort: 65000, altPortOffset: 1000 }] }), /field 'altPortOffset' makes canonicalPort \+ altPortOffset exceed 65535/);
});

test('loadConfig rejects out-of-range or non-integer timeouts', () => {
  assert.match(loadBad({ drainSeconds: 'soon', proxies: [] }), /field 'drainSeconds' must be an integer in 0\.\.=3600/);
  assert.match(loadBad({ drainSeconds: -1, proxies: [] }), /field 'drainSeconds'/);
  assert.match(loadBad({ drainSeconds: 3601, proxies: [] }), /field 'drainSeconds'/);
  assert.match(loadBad({ healthTimeoutSeconds: -5, proxies: [] }), /field 'healthTimeoutSeconds' must be an integer in 1\.\.=3600/);
  assert.match(loadBad({ healthTimeoutSeconds: 0, proxies: [] }), /field 'healthTimeoutSeconds'/);
  assert.match(loadBad({ lockTimeoutSeconds: -1, proxies: [] }), /field 'lockTimeoutSeconds' must be an integer in 1\.\.=86400/);
  assert.match(loadBad({ lockTimeoutSeconds: 86401, proxies: [] }), /field 'lockTimeoutSeconds'/);
  assert.match(loadBad({ lockTimeoutSeconds: 1.5, proxies: [] }), /field 'lockTimeoutSeconds'/);
});

test('loadConfig rejects bad hashFiles/hashEnv/backends/env shapes', () => {
  assert.match(loadBad({ proxies: [{ ...validProxy, hashFiles: 'x' }] }), /field 'hashFiles' must be an array/);
  assert.match(loadBad({ proxies: [{ ...validProxy, hashFiles: [42] }] }), /field 'hashFiles\[0\]'/);
  assert.match(loadBad({ proxies: [{ ...validProxy, hashEnv: 'HOME' }] }), /field 'hashEnv' must be an array/);
  assert.match(loadBad({ proxies: [{ ...validProxy, backends: {} }] }), /field 'backends' must be an array/);
  assert.match(loadBad({ proxies: [{ ...validProxy, backends: [{ name: 'b1' }] }] }), /field 'backends\[0\]\.env' is required/);
  assert.match(loadBad({ proxies: [{ ...validProxy, env: [] }] }), /field 'env' must be an object/);
  assert.match(loadBad({ proxies: [{ ...validProxy, env: { A: 1 } }] }), /field 'env\.A' must be a string/);
  assert.match(loadBad({ proxies: [{ ...validProxy, strategy: 'sideways' }] }), /field 'strategy' must be one of/);
});

test('reconcile starts, then no-ops on unchanged hash', async (t) => {
  const w = await makeWorld(t);
  const r1 = await w.run();
  assert.equal(r1.action, 'started');
  assert.equal(r1.port, w.canonicalPort);
  const h1 = await getHealth(w.canonicalPort);
  assert.equal(h1.status, 200);
  const pid1 = r1.pid;
  assert.ok(pidAlive(pid1));
  const r2 = await w.run();
  assert.equal(r2.action, 'noop');
  assert.equal(r2.pid, pid1, 'no-op must not touch the running process');
  const ep = JSON.parse(fs.readFileSync(path.join(w.endpointDir, 'fake-proxy', 'endpoint.json'), 'utf8'));
  assert.equal(ep.port, w.canonicalPort);
  assert.equal(ep.pid, pid1);
  assert.ok(ep.hash, 'endpoint record carries the hash');
});

test('hash change rolls A/B: new port serves, old drained, endpoint flipped', async (t) => {
  const w = await makeWorld(t);
  const r1 = await w.run();
  fs.writeFileSync(w.scriptPath, FAKE('v2'));
  const r2 = await w.run();
  assert.equal(r2.action, 'rolled');
  assert.equal(r2.port, w.altPort);
  assert.notEqual(r2.pid, r1.pid);
  assert.equal(pidAlive(r1.pid), false, 'old instance drained');
  assert.equal((await getHealth(w.canonicalPort)).status, 0, 'canonical no longer serving');
  const h = await getHealth(w.altPort);
  assert.equal(h.status, 200);
  assert.match(h.body, /v2/);
  const ep = JSON.parse(fs.readFileSync(path.join(w.endpointDir, 'fake-proxy', 'endpoint.json'), 'utf8'));
  assert.equal(ep.port, w.altPort);
  assert.equal(ep.previousPort, w.canonicalPort);
  assert.ok(ep.hash, 'endpoint record survives the roll in extended shape (old instance unlinks it on exit; supervisor rewrites)');
  // And back again on the next change (roles of the ports swap).
  fs.writeFileSync(w.scriptPath, FAKE('v3'));
  const r3 = await w.run();
  assert.equal(r3.action, 'rolled');
  assert.equal(r3.port, w.canonicalPort);
  assert.match((await getHealth(w.canonicalPort)).body, /v3/);
});

test('swap strategy validates on alt but settles on canonical', async (t) => {
  const w = await makeWorld(t, { strategy: 'swap' });
  const r1 = await w.run();
  assert.equal(r1.port, w.canonicalPort);
  fs.writeFileSync(w.scriptPath, FAKE('v2'));
  const r2 = await w.run();
  assert.equal(r2.action, 'rolled');
  assert.equal(r2.port, w.canonicalPort, 'swap settles on canonical');
  assert.equal(pidAlive(r1.pid), false);
  assert.match((await getHealth(w.canonicalPort)).body, /v2/);
  const r3 = await w.run();
  assert.equal(r3.action, 'noop');
  assert.equal(r3.pid, r2.pid);
});

test('broken new build keeps the old instance serving', async (t) => {
  const w = await makeWorld(t);
  const r1 = await w.run();
  fs.writeFileSync(w.scriptPath, 'process.exit(3);\n');
  const r2 = await w.run();
  assert.equal(r2.action, 'roll-failed-old-kept');
  assert.ok(pidAlive(r1.pid), 'old instance untouched');
  assert.match((await getHealth(w.canonicalPort)).body, /v1/);
});

test('foreign healthy instance is adopted, not duplicated', async (t) => {
  const w = await makeWorld(t);
  // Simulate a systemd-started instance: spawn the fake directly and
  // publish a fleet-shaped endpoint.json, like the real proxies do.
  const child = spawn(process.execPath, [w.scriptPath], {
    env: { ...process.env, FAKE_PROXY_PORT: String(w.canonicalPort) },
    detached: true,
    stdio: 'ignore',
  });
  child.unref();
  track(child.pid);
  const epDir = path.join(w.endpointDir, 'fake-proxy');
  fs.mkdirSync(epDir, { recursive: true });
  fs.writeFileSync(path.join(epDir, 'endpoint.json'), JSON.stringify({ host: '127.0.0.1', port: w.canonicalPort, pid: child.pid, url: `http://127.0.0.1:${w.canonicalPort}` }));
  for (let i = 0; i < 50 && (await getHealth(w.canonicalPort)).status !== 200; i++) {
    await new Promise((r) => setTimeout(r, 100));
  }
  const r = await w.run();
  assert.equal(r.action, 'adopt');
  assert.equal(r.pid, child.pid);
  const r2 = await w.run();
  assert.equal(r2.action, 'noop');
  assert.equal(r2.pid, child.pid);
});

test('module under test is the worktree copy', () => {
  assert.ok(fs.existsSync(path.join(here, 'proxy-supervise.mjs')));
});

test('CLI entry point still runs when invoked through a symlinked path (#2083)', async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'proxy-supervise-symlink-'));
  try {
    const link = path.join(root, 'proxy-supervise-link.mjs');
    fs.symlinkSync(path.join(here, 'proxy-supervise.mjs'), link);
    const cfgFile = path.join(root, 'config.json');
    fs.writeFileSync(
      cfgFile,
      JSON.stringify({
        stateDir: path.join(root, 'state'),
        endpointDir: path.join(root, 'endpoints'),
        proxies: [
          { name: 'ghost-proxy', canonicalPort: 19999, portEnv: 'GHOST_PROXY_PORT', command: ['/bin/true'], hashFiles: [] },
        ],
      }),
    );
    const out = await new Promise((resolve, reject) => {
      const child = spawn(process.execPath, [link, '--config', cfgFile, '--only', 'ghost-proxy', '--status']);
      let buf = '';
      child.stdout.on('data', (d) => (buf += d));
      child.on('error', reject);
      child.on('close', () => resolve(buf));
    });
    // With the old string-compare guard, main() never ran through a
    // symlink and this printed nothing at all.
    assert.match(out, /"name":"ghost-proxy"/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
