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

test('bytes sent during the tunnel CONNECT window are forwarded byte-for-byte in order', async (t) => {
  const sockets = new Set();
  const track = (socket) => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
    return socket;
  };
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'proxy-relay-test-'));

  // Recording sink: the CONNECT target. Collects raw bytes, replies nothing.
  let received = Buffer.alloc(0);
  let sinkSocketResolve;
  const sinkConnected = new Promise((resolve) => { sinkSocketResolve = resolve; });
  const sink = net.createServer((socket) => {
    track(socket);
    socket.on('data', (chunk) => { received = Buffer.concat([received, chunk]); });
    sinkSocketResolve(socket);
  });
  sink.on('connection', track);
  const sinkPort = await listen(sink);

  // Tunnel proxy that delays its CONNECT answer by 400ms, opening the
  // window in which an unpaused relay client used to lose body bytes.
  const CONNECT_DELAY_MS = 400;
  const tunnelProxy = net.createServer((client) => {
    track(client);
    let request = Buffer.alloc(0);
    const onData = (chunk) => {
      request = Buffer.concat([request, chunk]);
      if (request.indexOf('\r\n\r\n') === -1) return;
      client.removeListener('data', onData);
      client.pause();
      const upstream = track(net.connect(sinkPort, '127.0.0.1'));
      client.once('close', () => upstream.destroy());
      client.once('error', () => upstream.destroy());
      upstream.once('connect', () => {
        setTimeout(() => {
          if (client.destroyed) return;
          client.write('HTTP/1.1 200 Connection Established\r\n\r\n');
          client.pipe(upstream);
          upstream.pipe(client);
        }, CONNECT_DELAY_MS);
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
      RELAY_TARGET_PORTS: String(sinkPort),
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
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
    for (const socket of sockets) socket.destroy();
    await Promise.all([closeServer(tunnelProxy), closeServer(sink)]);
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

  // /health fast-path control: answered locally, no tunnel, no sink bytes.
  const health = await within(fetch(`http://127.0.0.1:${relayPort}/health`, {
    headers: { connection: 'close' }
  }), 5000, 'local /health');
  assert.equal(health.status, 200);
  assert.equal((await health.json()).relay, true);
  assert.equal(received.length, 0, '/health must not reach the sink');

  const openClient = async () => {
    const socket = track(net.createConnection({ host: '127.0.0.1', port: relayPort }));
    await within(new Promise((resolve, reject) => {
      socket.once('connect', resolve);
      socket.once('error', reject);
    }), 5000, 'relay client connection');
    return socket;
  };

  // Multi-segment request: headers first, then body chunks sent while the
  // tunnel CONNECT is still in flight. Every byte must arrive, in order.
  const head = 'POST /submit HTTP/1.1\r\nHost: sink\r\nContent-Length: 11\r\nConnection: close\r\n\r\n';
  const multi = await openClient();
  multi.write(head);
  await delay(100);
  multi.write('hello ');
  await delay(100);
  multi.write('world');
  await within(sinkConnected, 5000, 'sink connection');
  await delay(1500);
  assert.equal(
    received.toString('latin1'),
    `${head}hello world`,
    'bytes sent during the CONNECT window must follow the peeked buffer in order'
  );
  multi.destroy();

  // Single-segment control: one write is forwarded whole.
  received = Buffer.alloc(0);
  const single = 'GET /one-shot HTTP/1.1\r\nHost: sink\r\nConnection: close\r\n\r\n';
  const oneShot = await openClient();
  oneShot.write(single);
  await delay(1500);
  assert.equal(received.toString('latin1'), single);
  oneShot.destroy();
});

test('endpoint.json is republished when failover changes the preferred target', async (t) => {
  const sockets = new Set();
  const track = (socket) => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
    return socket;
  };
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'proxy-relay-test-'));
  const stateDir = path.join(tmp, 'state');

  const backend = http.createServer((req, res) => {
    res.writeHead(200, { connection: 'close' });
    res.end('live-backend');
  });
  backend.on('connection', track);
  const livePort = await listen(backend);
  const deadPort = await freePort(); // nothing listens here

  // Tunnel proxy: 503 for anything but the live target, real pipe for it.
  const tunnelProxy = net.createServer((client) => {
    track(client);
    let request = Buffer.alloc(0);
    const onData = (chunk) => {
      request = Buffer.concat([request, chunk]);
      const headerEnd = request.indexOf('\r\n\r\n');
      if (headerEnd === -1) return;
      client.removeListener('data', onData);
      const firstLine = request.slice(0, request.indexOf('\r\n')).toString('latin1');
      const targetPort = Number(firstLine.split(' ')[1].split(':').pop());
      if (targetPort !== livePort) {
        client.end('HTTP/1.1 503 Service Unavailable\r\n\r\n');
        return;
      }
      const upstream = track(net.connect(livePort, '127.0.0.1'));
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
      RELAY_TARGET_PORTS: `${deadPort},${livePort}`,
      RELAY_PROXY_HOST: '127.0.0.1',
      RELAY_PROXY_PORT: String(proxyPort),
      RELAY_NAME: 'proxy-relay-failover-test',
      XDG_STATE_HOME: stateDir
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
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
    for (const socket of sockets) socket.destroy();
    await Promise.all([closeServer(tunnelProxy), closeServer(backend)]);
    fs.rmSync(tmp, { recursive: true, force: true });
  });

  const ready = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`relay did not start:\n${log}`)), 10_000);
    child.stdout.on('data', () => {
      if (!log.includes('[proxy-relay-failover-test] relay http://')) return;
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

  const endpointFile = path.join(stateDir, 'proxy-relay-failover-test', 'endpoint.json');
  const readRelayTo = () => JSON.parse(fs.readFileSync(endpointFile, 'utf8')).relayTo;
  // The relay publishes endpoint.json from its listen callback, just after
  // its log line; on a loaded host the file can land a few ms after the log
  // line reaches this process, so poll briefly instead of reading once.
  // (Observed flaking ~50% on the fleet laptop, 2026-11-08, task 12671.)
  const readRelayToEventually = async (want) => {
    const deadline = Date.now() + 5000;
    for (;;) {
      try {
        const value = readRelayTo();
        if (want === undefined || value === want) return value;
      } catch { /* not published yet */ }
      if (Date.now() >= deadline) return readRelayTo();
      await delay(25);
    }
  };
  assert.equal(await readRelayToEventually(), `127.0.0.1:${deadPort}`, 'startup endpoint names the first target');

  // Force a failover with a real forwarded request (dead target first).
  const forwarded = await within(fetch(`http://127.0.0.1:${relayPort}/proxied`, {
    headers: { connection: 'close' }
  }), 5000, 'forwarded request');
  assert.equal(forwarded.status, 200);
  assert.equal(await forwarded.text(), 'live-backend');

  const health = await within(fetch(`http://127.0.0.1:${relayPort}/health`, {
    headers: { connection: 'close' }
  }), 5000, 'local /health');
  const { preferredTarget } = await health.json();
  assert.equal(preferredTarget, livePort, '/health must report the live target after failover');
  assert.equal(await readRelayToEventually(`127.0.0.1:${preferredTarget}`), `127.0.0.1:${preferredTarget}`, 'endpoint.json relayTo must agree with /health after failover');
});
