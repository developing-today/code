// Tests for design-code-proxy's local-dispatch fallback (#681).
// Run: node --test bin/design-code-proxy.test.mjs
//
// The proxy dispatches a chat request to a primary local backend and
// falls back to a second one on quota/5xx/connection failures. These
// tests run the real proxy against two in-process stub backends whose
// behaviour each test sets per stub (primary.mode / fallback.mode):
//
//   'quota-body'   -> HTTP 200 with {"error":{"code":429,...}} in the body
//   'quota-status' -> HTTP 429 status
//   'ok' (default) -> a successful chat completion naming the backend
//
// A quota error reported in a 200 JSON body used to hang the client
// request forever: tryTarget wrote the response head before reading
// the body, so the in-body quota check's onFail hit the headersSent
// guard and silently returned — no fallback, no response end.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import http from 'node:http';
import net from 'node:net';
import path from 'node:path';
import test from 'node:test';
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

const startStub = (name) => new Promise((resolve) => {
  const stub = { name, mode: 'ok', calls: 0, port: 0, stop: () => {} };
  const server = http.createServer((req, res) => {
    let bodyText = '';
    req.on('data', c => { bodyText += c; });
    req.on('end', () => {
      stub.calls += 1;
      if (stub.mode === 'quota-body') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { code: 429, message: `${name} quota`, type: 'insufficient_quota' } }));
      } else if (stub.mode === 'quota-status') {
        res.writeHead(429, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { code: 429, message: `${name} quota` } }));
      } else if (bodyText.includes('"stream":true')) {
        res.writeHead(200, { 'Content-Type': 'text/event-stream' });
        const chunk = {
          id: 'chatcmpl-stub', object: 'chat.completion.chunk', created: 0, model: 'stub-model',
          choices: [{ index: 0, delta: { content: `hello from ${name}` }, finish_reason: null }]
        };
        res.write(`data: ${JSON.stringify(chunk)}\n\n`);
        res.write('data: [DONE]\n\n');
        res.end();
      } else {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          id: 'chatcmpl-stub', object: 'chat.completion', created: 0, model: 'stub-model',
          choices: [{ index: 0, message: { role: 'assistant', content: `hello from ${name}` }, finish_reason: 'stop' }]
        }));
      }
    });
  });
  server.listen(0, '127.0.0.1', () => {
    stub.port = server.address().port;
    stub.stop = () => server.close();
    resolve(stub);
  });
});

const startProxy = async () => {
  const primary = await startStub('primary');
  const fallback = await startStub('fallback');
  const port = await freePort();
  const env = { ...process.env };
  delete env.OPEN_DESIGN_API_KEY;
  delete env.AMR_API_KEY;
  const child = spawn(process.execPath, [path.join(binDir, 'design-code-proxy.mjs')], {
    env: {
      ...env,
      DESIGN_CODE_PROXY_PORT: String(port),
      ANTIGRAVITY_LS_PROXY_PORT: String(primary.port),
      CODEX_PROXY_PORT: String(fallback.port)
    },
    stdio: ['ignore', 'pipe', 'pipe']
  });
  let log = '';
  child.stdout.on('data', d => { log += d; });
  child.stderr.on('data', d => { log += d; });
  const deadline = Date.now() + 10000;
  for (;;) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/health`);
      if (res.ok) break;
    } catch { /* not up yet */ }
    if (Date.now() > deadline) throw new Error(`proxy did not start:\n${log}`);
    await new Promise(r => setTimeout(r, 50));
  }
  return {
    port, primary, fallback,
    stop: () => { child.kill('SIGKILL'); primary.stop(); fallback.stop(); }
  };
};

// design-code-flash: primary = Antigravity stub, fallback = Codex stub.
// The 5s abort makes the pre-fix hang fail fast instead of wedging the suite.
const post = (port, extra = {}) => fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ model: 'design-code-flash', messages: [{ role: 'user', content: 'hi' }], ...extra }),
  signal: AbortSignal.timeout(5000)
});

test('successful primary response is unchanged (model rewritten, no fallback call)', async (t) => {
  const { port, primary, fallback, stop } = await startProxy();
  t.after(stop);
  const res = await post(port);
  assert.equal(res.status, 200);
  const body = JSON.parse(await res.text());
  assert.equal(body.choices[0].message.content, 'hello from primary');
  assert.equal(body.model, 'design-code-flash');
  assert.equal(primary.calls, 1);
  assert.equal(fallback.calls, 0);
});

test('quota error in a 200 JSON body falls back instead of hanging (#681)', async (t) => {
  const { port, primary, fallback, stop } = await startProxy();
  t.after(stop);
  primary.mode = 'quota-body';
  const res = await post(port);
  assert.equal(res.status, 200);
  const body = JSON.parse(await res.text());
  assert.equal(body.choices[0].message.content, 'hello from fallback');
  assert.equal(body.model, 'design-code-flash');
  assert.equal(primary.calls, 1);
  assert.equal(fallback.calls, 1);
});

test('quota error in a 200 JSON body on the fallback target answers 502 promptly (#681)', async (t) => {
  const { port, primary, fallback, stop } = await startProxy();
  t.after(stop);
  primary.mode = 'quota-body';
  fallback.mode = 'quota-body';
  const res = await post(port);
  assert.equal(res.status, 502);
  const body = JSON.parse(await res.text());
  assert.match(body.error.message, /All design code backends failed/);
  assert.equal(primary.calls, 1);
  assert.equal(fallback.calls, 1);
});

test('HTTP-status 429 still falls back (control)', async (t) => {
  const { port, primary, fallback, stop } = await startProxy();
  t.after(stop);
  primary.mode = 'quota-status';
  const res = await post(port);
  assert.equal(res.status, 200);
  const body = JSON.parse(await res.text());
  assert.equal(body.choices[0].message.content, 'hello from fallback');
  assert.equal(fallback.calls, 1);
});

test('streaming success is unchanged', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  const res = await post(port, { stream: true });
  assert.equal(res.status, 200);
  const sse = await res.text();
  assert.match(sse, /hello from primary/);
  assert.ok(sse.trimEnd().endsWith('data: [DONE]'));
});
