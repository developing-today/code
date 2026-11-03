// Tests for systemd socket activation in the provider proxies (#1821).
// Run: node --test bin/socket-activation.test.mjs
//
// Unit level: the LISTEN_FDS/LISTEN_PID parsing in socket-activation.mjs.
// Integration level: each proxy is spawned the way systemd starts it —
// a listening socket handed over as fd 3 with LISTEN_FDS=1 — and must
// answer on that socket (it never binds the port itself); the existing
// per-proxy test files cover the direct-launch fallback, where the proxy
// binds its configured port with no LISTEN_FDS in the environment.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { listenTarget, systemdActivationFd } from './socket-activation.mjs';

const binDir = path.dirname(fileURLToPath(import.meta.url));

test('systemdActivationFd parses the LISTEN_FDS protocol', () => {
  assert.equal(systemdActivationFd({}, 123), null);
  assert.equal(systemdActivationFd({ LISTEN_FDS: '0' }, 123), null);
  assert.equal(systemdActivationFd({ LISTEN_FDS: 'x' }, 123), null);
  assert.equal(systemdActivationFd({ LISTEN_FDS: '1' }, 123), 3);
  assert.equal(systemdActivationFd({ LISTEN_FDS: '2' }, 123), 3);
  assert.equal(systemdActivationFd({ LISTEN_FDS: '1', LISTEN_PID: '123' }, 123), 3);
  // An environment leaked from another process's activation is ignored.
  assert.equal(systemdActivationFd({ LISTEN_FDS: '1', LISTEN_PID: '999' }, 123), null);
});

test('listenTarget binds directly without activation, uses fd 3 with it', () => {
  assert.deepEqual(listenTarget({ port: 18742, host: '127.0.0.1' }, {}, 123), { port: 18742, host: '127.0.0.1' });
  assert.deepEqual(listenTarget({ port: 18742, host: '127.0.0.1' }, { LISTEN_FDS: '1' }, 123), { fd: 3 });
});

// Spawn `script` with a listening socket as fd 3, the way a systemd
// .socket unit starts its service, and wait for its /health route.
const startSocketActivated = async (script, extraEnv) => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'socket-activation-test-'));
  const srv = net.createServer();
  await new Promise((resolve, reject) => {
    srv.once('error', reject);
    srv.listen(0, '127.0.0.1', resolve);
  });
  const { port } = srv.address();
  const fd = srv._handle.fd;
  const env = {
    ...process.env,
    HOME: tmp,
    XDG_STATE_HOME: path.join(tmp, 'state'),
    LISTEN_FDS: '1',
    ...extraEnv(port),
  };
  delete env.LISTEN_PID;
  const child = spawn(process.execPath, [path.join(binDir, script)], {
    env,
    stdio: ['ignore', 'pipe', 'pipe', fd],
  });
  // The descriptor is dup'd into the child; the parent copy can close.
  srv.close();
  let log = '';
  child.stdout.on('data', (d) => { log += d; });
  child.stderr.on('data', (d) => { log += d; });
  const deadline = Date.now() + 10000;
  for (;;) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/health`);
      if (res.ok) return { child, port, log: () => log };
    } catch { /* not up yet */ }
    if (child.exitCode !== null) throw new Error(`${script} exited early (${child.exitCode}):\n${log}`);
    if (Date.now() > deadline) throw new Error(`${script} did not answer on the handed-off socket:\n${log}`);
    await new Promise((r) => setTimeout(r, 50));
  }
};

test('clef-proxy serves on the systemd handed-off listener', async (t) => {
  const { child, log } = await startSocketActivated('clef-proxy.mjs', (port) => ({
    CLEF_PROXY_PORT: String(port),
  }));
  t.after(() => { child.kill('SIGTERM'); });
  assert.match(log(), /listening on/);
});

test('codex-proxy serves on the systemd handed-off listener', async (t) => {
  const { child, log } = await startSocketActivated('codex-proxy.mjs', (port) => ({
    CODEX_PROXY_PORT: String(port),
    CODEX_PROXY_TOKEN_SYNC: '0',
    CODEX_PROXY_CODEX_BIN: '/bin/true',
  }));
  t.after(() => { child.kill('SIGTERM'); });
  assert.match(log(), /listening on/);
});
