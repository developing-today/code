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
//   'ok-length'    -> a successful chat completion with an explicit
//                     Content-Length on the compact JSON body (#684)
//   'ok-length-stream' -> the SSE stream, with a Content-Length set to
//                     the backend's own (pre-model-rewrite) byte count,
//                     which is stale the moment the proxy rewrites the
//                     model in each chunk (#688)
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
  const stub = { name, mode: 'ok', calls: 0, port: 0, remotePorts: [], stop: () => {} };
  const server = http.createServer((req, res) => {
    let bodyText = '';
    req.on('data', c => { bodyText += c; });
    req.on('end', () => {
      stub.calls += 1;
      stub.remotePorts.push(req.socket.remotePort);
      if (stub.mode === 'error-mid-stream') {
        // Send a head and a partial body, then destroy the socket:
        // the proxy's backend response errors mid-stream (#782).
        if (bodyText.includes('"stream":true')) {
          res.writeHead(200, { 'Content-Type': 'text/event-stream' });
          const chunk = {
            id: 'chatcmpl-stub', object: 'chat.completion.chunk', created: 0, model: 'stub-model',
            choices: [{ index: 0, delta: { content: `partial from ${name}` }, finish_reason: null }]
          };
          res.write(`data: ${JSON.stringify(chunk)}\n\n`);
        } else {
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.write('{"id":"chatcmpl-stub","partial":');
        }
        setTimeout(() => { res.socket.destroy(); }, 25);
        return;
      }
      if (stub.mode === 'quota-body') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { code: 429, message: `${name} quota`, type: 'insufficient_quota' } }));
      } else if (stub.mode === 'quota-status') {
        res.writeHead(429, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { code: 429, message: `${name} quota` } }));
      } else if (stub.mode === 'ok-length' && !bodyText.includes('"stream":true')) {
        const compact = JSON.stringify({
          id: 'chatcmpl-stub', object: 'chat.completion', created: 0, model: 'stub-model',
          choices: [{ index: 0, message: { role: 'assistant', content: `hello from ${name}` }, finish_reason: 'stop' }]
        });
        res.writeHead(200, { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(compact) });
        res.end(compact);
      } else if (bodyText.includes('"stream":true')) {
        const chunk = {
          id: 'chatcmpl-stub', object: 'chat.completion.chunk', created: 0, model: 'stub-model',
          choices: [{ index: 0, delta: { content: `hello from ${name}` }, finish_reason: null }]
        };
        const payload = `data: ${JSON.stringify(chunk)}\n\ndata: [DONE]\n\n`;
        const headers = { 'Content-Type': 'text/event-stream' };
        if (stub.mode === 'ok-length-stream') {
          // #688: the length of the backend's own body. The proxy's
          // per-chunk model rewrite makes the body it sends longer, so
          // forwarding this length would truncate the stream.
          headers['Content-Length'] = Buffer.byteLength(payload);
        }
        res.writeHead(200, headers);
        res.end(payload);
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

test('backend Content-Length does not truncate the re-serialized success body (#684)', async (t) => {
  const { port, primary, fallback, stop } = await startProxy();
  t.after(stop);
  primary.mode = 'ok-length';
  const res = await post(port);
  assert.equal(res.status, 200);
  const text = await res.text();
  // The forwarded head must describe the body actually sent: either no
  // Content-Length (chunked) or exactly the received byte count — never
  // the backend's shorter compact-body length.
  const advertised = res.headers.get('content-length');
  if (advertised !== null) {
    assert.equal(Number(advertised), Buffer.byteLength(text));
  }
  const body = JSON.parse(text);
  assert.equal(body.choices[0].message.content, 'hello from primary');
  assert.equal(body.model, 'design-code-flash');
  assert.equal(primary.calls, 1);
  assert.equal(fallback.calls, 0);
});

test('backend Content-Length does not truncate the model-rewritten SSE stream (#688)', async (t) => {
  const { port, primary, fallback, stop } = await startProxy();
  t.after(stop);
  primary.mode = 'ok-length-stream';
  const res = await post(port, { stream: true });
  assert.equal(res.status, 200);
  const sse = await res.text();
  // The proxy rewrites the model in every chunk, so the stream it sends
  // is longer than the backend body the stub's Content-Length describes.
  // Forwarding that stale length truncates the tail — [DONE] is lost.
  assert.match(sse, /hello from primary/);
  assert.match(sse, /design-code-flash/);
  assert.ok(sse.trimEnd().endsWith('data: [DONE]'), `stream truncated: ${JSON.stringify(sse)}`);
  // The forwarded head must describe the stream actually sent: either no
  // Content-Length (chunked) or exactly the received byte count.
  const advertised = res.headers.get('content-length');
  if (advertised !== null) {
    assert.equal(Number(advertised), Buffer.byteLength(sse));
  }
  assert.equal(primary.calls, 1);
  assert.equal(fallback.calls, 0);
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

// Keep-alive pool + response-error contracts (#768, per #782). The
// stub records the source port of each backend connection: a reused
// pooled socket arrives from the same port, a fresh handshake from a
// new one.
test('sequential requests reuse the keep-alive pool (#782)', async (t) => {
  const { port, primary, fallback, stop } = await startProxy();
  t.after(stop);
  const first = await post(port);
  assert.equal(first.status, 200);
  await first.text();
  const second = await post(port);
  assert.equal(second.status, 200);
  await second.text();
  assert.equal(primary.calls, 2);
  assert.equal(fallback.calls, 0);
  assert.equal(primary.remotePorts.length, 2);
  assert.equal(primary.remotePorts[0], primary.remotePorts[1],
    `expected one pooled socket, saw source ports ${primary.remotePorts}`);
});

test('backend response error before headers falls back (#782)', async (t) => {
  const { port, primary, fallback, stop } = await startProxy();
  t.after(stop);
  primary.mode = 'error-mid-stream';
  // Non-stream: the proxy sends no head until the backend body is
  // complete, so a mid-stream backend error is still "before headers"
  // for the client and must fall back, not hang or tear down.
  const res = await post(port);
  assert.equal(res.status, 200);
  const body = JSON.parse(await res.text());
  assert.equal(body.choices[0].message.content, 'hello from fallback');
  assert.equal(primary.calls, 1);
  assert.equal(fallback.calls, 1);
});

test('backend response error after headers tears the client response down (#782)', async (t) => {
  const { port, primary, fallback, stop } = await startProxy();
  t.after(stop);
  primary.mode = 'error-mid-stream';
  // Stream: the head is written up front, so a mid-stream backend
  // error is "after headers" — the client response must be destroyed
  // promptly (its body read fails), never left hanging and never
  // completed as if the stream had ended cleanly.
  const res = await post(port, { stream: true });
  assert.equal(res.status, 200);
  const startedAt = Date.now();
  await assert.rejects(res.text());
  // The rejection must come from the proxy tearing the response down,
  // not from the client's own 5s abort: a proxy that simply hangs
  // would also reject, but only at the abort deadline.
  assert.ok(Date.now() - startedAt < 3000,
    `client response was not torn down promptly (${Date.now() - startedAt}ms)`);
  assert.equal(primary.calls, 1);
  assert.equal(fallback.calls, 0);
});
