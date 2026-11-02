// Loopback integration tests for the proxy relay.
// Run: node --test bin/proxy-relay.test.mjs
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const binDir = path.dirname(fileURLToPath(import.meta.url));
const FIRST_LINE_TIMEOUT_MS = 10_000;

const listen = (server, port = 0) => new Promise((resolve, reject) => {
  server.once('error', reject);
  server.listen(port, '127.0.0.1', () => resolve(server.address().port));
});

const freePort = () => new Promise((resolve, reject) => {
  const probe = net.createServer();
  probe.once('error', reject);
  probe.listen(0, '127.0.0.1', () => {
    const { port } = probe.address();
    probe.close((err) => err ? reject(err) : resolve(port));
  });
});

const within = (promise, ms, description) => {
  let timer;
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`timed out waiting for ${description}`)), ms);
    })
  ]).finally(() => clearTimeout(timer));
};

const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

test('first-line deadline reaps idle and partial clients while health and forwarded pipes work', async (t) => {
  const sockets = new Set();
  const track = (socket) => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
    return socket;
  };
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'proxy-relay-test-'));
  let delayedResponseTimer;
  let slowRequestStartedResolve;
  const slowRequestStarted = new Promise((resolve) => { slowRequestStartedResolve = resolve; });
  const backend = http.createServer((req, res) => {
    if (req.url === '/slow') {
      slowRequestStartedResolve();
      delayedResponseTimer = setTimeout(() => {
        res.writeHead(200, { connection: 'close' });
        res.end('forwarded');
      }, FIRST_LINE_TIMEOUT_MS + 500);
      return;
    }
    res.writeHead(200, { connection: 'close' });
    res.end('forwarded');
  });
  backend.on('connection', track);
  const backendPort = await listen(backend);

  const connectRequests = [];
  const tunnelProxy = net.createServer((client) => {
    track(client);
    let request = Buffer.alloc(0);
    const onData = (chunk) => {
      request = Buffer.concat([request, chunk]);
      const headerEnd = request.indexOf('\r\n\r\n');
      if (headerEnd === -1) return;
      client.removeListener('data', onData);
      connectRequests.push(request.slice(0, headerEnd).toString('latin1'));
      const upstream = track(net.connect(backendPort, '127.0.0.1'));
      client.once('close', () => upstream.destroy());
      client.once('error', () => upstream.destroy());
      upstream.once('connect', () => {
        client.write('HTTP/1.1 200 Connection Established\r\n\r\n');
        client.pipe(upstream);
        upstream.pipe(client);
      });
      upstream.once('error', () => client.destroy());
    };
    client.on('data', onData);
  });
  const proxyPort = await listen(tunnelProxy);

  const relayPort = await freePort();
  const child = spawn(process.execPath, [path.join(binDir, 'proxy-relay.mjs')], {
    env: {
      ...process.env,
      RELAY_LISTEN_HOST: '127.0.0.1',
      RELAY_LISTEN_PORT: String(relayPort),
      RELAY_TARGET_HOST: '127.0.0.1',
      RELAY_TARGET_PORTS: String(backendPort),
      RELAY_PROXY_HOST: '127.0.0.1',
      RELAY_PROXY_PORT: String(proxyPort),
      RELAY_NAME: 'proxy-relay-test',
      XDG_STATE_HOME: path.join(tmp, 'state')
    },
    stdio: ['ignore', 'pipe', 'pipe']
  });
  let log = '';
  child.stdout.on('data', (chunk) => { log += chunk; });
  child.stderr.on('data', (chunk) => { log += chunk; });

  const closeServer = (server) => new Promise((resolve) => {
    if (!server.listening) return resolve();
    server.close(() => resolve());
  });
  t.after(async () => {
    if (delayedResponseTimer) clearTimeout(delayedResponseTimer);
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
    for (const socket of sockets) socket.destroy();
    await Promise.all([closeServer(tunnelProxy), closeServer(backend)]);
    fs.rmSync(tmp, { recursive: true, force: true });
  });

  const ready = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`relay did not start:\n${log}`)), 10_000);
    child.stdout.on('data', () => {
      if (!log.includes('[proxy-relay-test] relay http://')) return;
      clearTimeout(timer);
      resolve();
    });
    child.once('error', (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.once('exit', (code, signal) => {
      clearTimeout(timer);
      reject(new Error(`relay exited before listening (${code ?? signal}):\n${log}`));
    });
  });
  await ready;

  const health = await within(fetch(`http://127.0.0.1:${relayPort}/health`, {
    headers: { connection: 'close' }
  }), 5000, 'local /health');
  assert.equal(health.status, 200);
  assert.equal((await health.json()).relay, true);
  assert.equal(connectRequests.length, 0, '/health should not open a tunnel');

  const forwardedPromise = within(fetch(`http://127.0.0.1:${relayPort}/slow`, {
    headers: { connection: 'close' }
  }), FIRST_LINE_TIMEOUT_MS + 5000, 'forwarded response');
  await within(slowRequestStarted, 5000, 'forwarded request reaching the backend');

  const openClient = async () => {
    const socket = track(net.createConnection({ host: '127.0.0.1', port: relayPort }));
    await within(new Promise((resolve, reject) => {
      socket.once('connect', resolve);
      socket.once('error', reject);
    }), 5000, 'relay client connection');
    return socket;
  };
  const [idle, partial] = await Promise.all([openClient(), openClient()]);
  const openedAt = Date.now();
  const closedAt = (socket) => new Promise((resolve) => socket.once('close', () => resolve(Date.now())));
  const idleClosed = closedAt(idle);
  const partialClosed = closedAt(partial);
  partial.write('GET /hea');

  await delay(5000);
  assert.equal(idle.destroyed, false, 'idle client must remain open before the deadline');
  assert.equal(partial.destroyed, false, 'partial-line client must remain open before the deadline');
  partial.write('lth'); // trickle data without completing CRLF; this must not reset the absolute deadline

  const [idleClosedAt, partialClosedAt] = await within(
    Promise.all([idleClosed, partialClosed]),
    7500,
    'idle and partial-line connections to be reaped'
  );
  for (const [name, closed] of [['idle', idleClosedAt], ['partial-line', partialClosedAt]]) {
    const elapsed = closed - openedAt;
    assert.ok(elapsed >= FIRST_LINE_TIMEOUT_MS - 1500, `${name} closed too early at ${elapsed}ms`);
    assert.ok(elapsed <= FIRST_LINE_TIMEOUT_MS + 2500, `${name} closed too late at ${elapsed}ms`);
  }
  assert.equal(connectRequests.length, 1, 'idle and partial-line clients must not open tunnels');
  assert.match(connectRequests[0], /^CONNECT 127\.0\.0\.1:\d+ HTTP\/1\.1/);

  const forwarded = await forwardedPromise;
  assert.equal(forwarded.status, 200);
  assert.equal(await forwarded.text(), 'forwarded');
});
