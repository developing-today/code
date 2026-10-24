// Tests for antigravity-ls-proxy's model resolution (#686).
// Run: node --test bin/antigravity-ls-proxy.test.mjs
//
// The proxy talks to an Antigravity language server over HTTP RPC. These
// tests run a stub language server, point the proxy at it via
// ANTIGRAVITY_LS_ADDRESS / ANTIGRAVITY_CSRF_TOKEN (the env attach path in
// detectRunningInstances), and capture the `model` value the proxy sends
// in GetModelResponse. A MODEL_MAP family entry carries metadata keys
// (`default`, `name`) besides effort -> enum, and an effort string that
// collides with one of those keys used to "resolve" to the metadata
// value — an effort name or a display name — instead of a model enum.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import test, { after, before } from 'node:test';
import { fileURLToPath } from 'node:url';

const binDir = path.dirname(fileURLToPath(import.meta.url));

const freePort = () => new Promise((resolve, reject) => {
  const srv = net.createServer();
  srv.once('error', reject);
  srv.listen(0, '127.0.0.1', () => {
    const { port } = srv.address();
    srv.close(() => resolve(port));
  });
});

let lsServer;
let proxyChild;
let proxyPort;
let capturedModel;

const startStubLs = async () => {
  lsServer = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => { body += c; });
    req.on('end', () => {
      res.writeHead(200, { 'Content-Type': 'application/json' });
      if (req.url.endsWith('/GetAvailableModels')) {
        res.end(JSON.stringify({ response: { models: {} } }));
      } else if (req.url.endsWith('/GetModelResponse')) {
        try { capturedModel = JSON.parse(body).model; } catch { capturedModel = null; }
        res.end(JSON.stringify({ response: 'stub answer' }));
      } else {
        // Heartbeat and anything else: a bare 200 is all the proxy checks.
        res.end('{}');
      }
    });
  });
  const port = await freePort();
  await new Promise((resolve) => lsServer.listen(port, '127.0.0.1', resolve));
  return port;
};

before(async () => {
  const lsPort = await startStubLs();
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'ls-proxy-test-'));
  proxyPort = await freePort();
  proxyChild = spawn(process.execPath, [path.join(binDir, 'antigravity-ls-proxy.mjs')], {
    env: {
      ...process.env,
      ANTIGRAVITY_LS_PROXY_PORT: String(proxyPort),
      ANTIGRAVITY_LS_ADDRESS: `127.0.0.1:${lsPort}`,
      ANTIGRAVITY_CSRF_TOKEN: 'test-token',
      XDG_STATE_HOME: path.join(tmp, 'state')
    },
    stdio: ['ignore', 'pipe', 'pipe']
  });
  let log = '';
  proxyChild.stdout.on('data', (d) => { log += d; });
  proxyChild.stderr.on('data', (d) => { log += d; });
  const deadline = Date.now() + 10000;
  for (;;) {
    try {
      const res = await fetch(`http://127.0.0.1:${proxyPort}/health`);
      if (res.ok) break;
    } catch {}
    if (Date.now() > deadline) throw new Error(`proxy did not start: ${log}`);
    await new Promise((r) => setTimeout(r, 100));
  }
});

after(() => {
  if (proxyChild) proxyChild.kill('SIGKILL');
  if (lsServer) lsServer.close();
});

const chat = async (payload) => {
  capturedModel = undefined;
  const res = await fetch(`http://127.0.0.1:${proxyPort}/v1/chat/completions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      messages: [{ role: 'user', content: 'hello' }],
      ...payload
    })
  });
  return res;
};

test("effort 'default' falls back to the family default enum, not the string 'medium'", async () => {
  const res = await chat({ model: 'gemini-3.8-flash', reasoning_effort: 'default' });
  assert.equal(res.status, 200);
  assert.equal(capturedModel, 'MODEL_PLACEHOLDER_M319');
});

test("effort 'name' falls back to the family default enum, not the display name", async () => {
  const res = await chat({ model: 'gemini-3.8-flash', reasoning_effort: 'name' });
  assert.equal(res.status, 200);
  assert.equal(capturedModel, 'MODEL_PLACEHOLDER_M319');
});

test('a valid effort still resolves to its own enum', async () => {
  const res = await chat({ model: 'gemini-3.8-flash', reasoning_effort: 'high' });
  assert.equal(res.status, 200);
  assert.equal(capturedModel, 'MODEL_PLACEHOLDER_M318');
});

test('an unoffered effort still falls back to the family default', async () => {
  const res = await chat({ model: 'gemini-3.1-pro', reasoning_effort: 'medium' });
  assert.equal(res.status, 200);
  assert.equal(capturedModel, 'MODEL_PLACEHOLDER_M37');
});

test('an unknown model is still refused with 400', async () => {
  const res = await chat({ model: 'no-such-model-xyz' });
  assert.equal(res.status, 400);
  assert.equal(capturedModel, undefined);
});
