#!/usr/bin/env node
// proxy-relay.mjs — sandbox-side loopback relay to a laptop-hosted
// fleet proxy, through this VM's tailnet tunnel proxy.
//
// The sandbox has no direct tailnet route; the only path to the laptop
// (100.99.113.103) is the runtime tunnel proxy (the $HTTPS_PROXY host
// with its port changed to 3130), which answers UNAUTHENTICATED from
// inside the sandbox (verified 2026-10-24 against the laptop's
// antigravity-ls/codex/design proxies' /health).
//
// The relay is a raw TCP forwarder: accept on loopback, CONNECT through
// the tunnel proxy to the target, then pipe bytes. HTTP, SSE and
// websockets all pass through untouched. Targets are tried in order:
// the first target that accepts a CONNECT wins, and the winner is
// remembered until it fails. NOTE (measured 2026-10-24): this VM's
// tunnel only reaches the canonical fleet ports — CONNECTs to the
// +1000 alternates are refused proxy-side in ~0.1-0.5s even when a
// service is live there, so the sandbox config lists canonical
// targets only; a laptop proxy sitting on its alternate port is
// unreachable from the sandbox until its next roll flips it home.
//
// Env:
//   RELAY_LISTEN_PORT   (required) loopback port to listen on
//   RELAY_LISTEN_HOST   default 127.0.0.1
//   RELAY_TARGET_HOST   default 100.99.113.103 (the laptop)
//   RELAY_TARGET_PORTS  comma list, e.g. "18745,19745"
//   RELAY_PROXY_HOST    tunnel proxy host (default hatch-egress-proxy)
//   RELAY_PROXY_PORT    tunnel proxy port (default 3130)
//   RELAY_NAME          label for logs/endpoint (default "relay")
//
// Like the fleet proxies, it publishes <name>/endpoint.json under
// $XDG_STATE_HOME (default ~/.local/state) on listen.
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { parseProxyPort } from './proxy-port.mjs';

const LISTEN_PORT = parseProxyPort('RELAY_LISTEN_PORT', 0);
if (!LISTEN_PORT) {
  console.error('RELAY_LISTEN_PORT is required');
  process.exit(1);
}
const LISTEN_HOST = process.env.RELAY_LISTEN_HOST || '127.0.0.1';
const TARGET_HOST = process.env.RELAY_TARGET_HOST || '100.99.113.103';
const TARGET_PORTS = (process.env.RELAY_TARGET_PORTS || '')
  .split(',')
  .map((s) => Number(s.trim()))
  .filter((n) => Number.isInteger(n) && n >= 1 && n <= 65535);
if (TARGET_PORTS.length === 0) {
  console.error('RELAY_TARGET_PORTS is required (comma list)');
  process.exit(1);
}
const PROXY_HOST = process.env.RELAY_PROXY_HOST || 'hatch-egress-proxy';
const PROXY_PORT = parseProxyPort('RELAY_PROXY_PORT', 3130);
const NAME = process.env.RELAY_NAME || `relay-${LISTEN_PORT}`;

let preferred = TARGET_PORTS[0];

const publishEndpoint = () => {
  try {
    const dir = path.join(
      process.env.XDG_STATE_HOME || path.join(os.homedir(), '.local', 'state'),
      NAME
    );
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, 'endpoint.json');
    const tmp = `${file}.tmp`;
    fs.writeFileSync(
      tmp,
      JSON.stringify({
        host: LISTEN_HOST,
        port: LISTEN_PORT,
        pid: process.pid,
        url: `http://${LISTEN_HOST}:${LISTEN_PORT}`,
        relayTo: `${TARGET_HOST}:${preferred}`,
      }) + '\n'
    );
    fs.renameSync(tmp, file);
  } catch { /* best effort */ }
};

// One CONNECT attempt; resolves with the connected socket positioned
// just past the proxy's response headers.
const connectViaTunnel = (targetPort) =>
  new Promise((resolve, reject) => {
    const sock = net.connect(PROXY_PORT, PROXY_HOST);
    let buf = Buffer.alloc(0);
    const fail = (e) => {
      sock.destroy();
      reject(e);
    };
    sock.setTimeout(10000, () => fail(new Error('tunnel connect timeout')));
    sock.on('error', fail);
    sock.on('connect', () => {
      sock.write(
        `CONNECT ${TARGET_HOST}:${targetPort} HTTP/1.1\r\n` +
        `Host: ${TARGET_HOST}:${targetPort}\r\n\r\n`
      );
    });
    sock.on('data', (chunk) => {
      buf = Buffer.concat([buf, chunk]);
      const headEnd = buf.indexOf('\r\n\r\n');
      if (headEnd === -1) {
        if (buf.length > 16384) fail(new Error('tunnel response too large'));
        return;
      }
      const statusLine = buf.slice(0, buf.indexOf('\r\n')).toString('latin1');
      const status = Number(statusLine.split(' ')[1]);
      if (status !== 200) {
        fail(new Error(`tunnel CONNECT -> ${statusLine}`));
        return;
      }
      sock.removeAllListeners('data');
      sock.removeAllListeners('error');
      sock.setTimeout(0);
      const rest = buf.slice(headEnd + 4);
      if (rest.length) sock.unshift(rest);
      resolve(sock);
    });
  });

const tryTargets = async (ports) => {
  let lastErr = null;
  for (const p of ports) {
    try {
      const sock = await connectViaTunnel(p);
      preferred = p;
      return sock;
    } catch (e) {
      lastErr = e;
    }
  }
  throw lastErr || new Error('no target reachable');
};

// The supervisor health-checks the port this relay listens on; that
// check must measure the RELAY (is the forwarder up?), not the laptop
// backend — otherwise a backend outage makes the supervisor kill and
// respawn a perfectly good relay forever, and a laptop-side A/B flip
// (which the sandbox tunnel cannot follow — the tunnel only reaches
// the canonical fleet ports; the +1000 alternates are refused) would
// take the local listener down with it. So GET /health is answered
// locally; everything else is forwarded untouched.
const LOCAL_HEALTH = (reqLine) => /^GET \/health(\?\S*)? HTTP\//.test(reqLine);

const server = net.createServer((client) => {
  client.on('error', () => client.destroy());
  // Peek at the first request line before connecting upstream.
  let buf = Buffer.alloc(0);
  const onData = (chunk) => {
    buf = Buffer.concat([buf, chunk]);
    const headEnd = buf.indexOf('\r\n');
    if (headEnd === -1 && buf.length < 8192) return;
    client.removeListener('data', onData);
    const firstLine = buf.slice(0, headEnd === -1 ? buf.length : headEnd).toString('latin1');
    if (LOCAL_HEALTH(firstLine)) {
      client.end(
        'HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n' +
        JSON.stringify({
          status: 'ok',
          service: NAME,
          relay: true,
          listenPort: LISTEN_PORT,
          target: `${TARGET_HOST}:${TARGET_PORTS.join('|')}`,
          preferredTarget: preferred,
        })
      );
      return;
    }
    const order = [preferred, ...TARGET_PORTS.filter((p) => p !== preferred)];
    tryTargets(order)
      .then((upstream) => {
        upstream.on('error', () => { client.destroy(); upstream.destroy(); });
        upstream.write(buf);
        client.pipe(upstream);
        upstream.pipe(client);
      })
      .catch(() => client.destroy());
  };
  client.on('data', onData);
});

server.listen(LISTEN_PORT, LISTEN_HOST, () => {
  console.log(`[${NAME}] relay http://${LISTEN_HOST}:${LISTEN_PORT} -> ${TARGET_HOST}:${TARGET_PORTS.join('|')} via ${PROXY_HOST}:${PROXY_PORT}`);
  publishEndpoint();
});
