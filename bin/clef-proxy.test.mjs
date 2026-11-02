// Tests for clef-proxy's request-error handling (#690).
// Run: node --test bin/clef-proxy.test.mjs
//
// The local tests cover the paths that answer locally — health, a
// malformed client body (400, not the pre-fix 500 from the shared
// upstream try/catch), and a missing inference token (401, with HOME
// pointed at an empty dir and no token in the environment). The
// upstream tests below reach a stub gateway instead, via the
// CLEF_PROXY_GATEWAY_BASE seam (#692): they pin the #690 upstream
// guard — a non-JSON, null, or scalar gateway answer is an upstream
// failure preserving the upstream status, never a 500.
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

const freePort = () => new Promise((resolve, reject) => {
  const srv = net.createServer();
  srv.once('error', reject);
  srv.listen(0, '127.0.0.1', () => {
    const { port } = srv.address();
    srv.close(() => resolve(port));
  });
});

const startProxy = async (extraEnv = {}) => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'clef-proxy-test-'));
  const port = await freePort();
  const env = { ...process.env, HOME: tmp, CLEF_PROXY_PORT: String(port), XDG_STATE_HOME: path.join(tmp, 'state'), ...extraEnv };
  delete env.CLOUDFLARE_API_TOKEN;
  if (!extraEnv.CLEF_PROXY_GATEWAY_BASE) delete env.CLEF_PROXY_GATEWAY_BASE;
  const child = spawn(process.execPath, [path.join(binDir, 'clef-proxy.mjs')], {
    env,
    stdio: ['ignore', 'pipe', 'pipe']
  });
  let log = '';
  child.stdout.on('data', d => { log += d; });
  child.stderr.on('data', d => { log += d; });
  const deadline = Date.now() + 10000;
  for (;;) {
    if (log.includes('listening on')) break;
    if (Date.now() > deadline) throw new Error(`proxy did not start: ${log}`);
    await new Promise(r => setTimeout(r, 50));
  }
  return { child, base: `http://127.0.0.1:${port}`, tmp };
};

test('clef-proxy local error handling', async (t) => {
  const { child, base } = await startProxy();
  t.after(() => { child.kill('SIGTERM'); });

  await t.test('health answers 200', async () => {
    const res = await fetch(`${base}/health`);
    assert.equal(res.status, 200);
    const data = await res.json();
    assert.equal(data.status, 'ok');
  });

  await t.test('invalid JSON body is a 400, not a 500', async () => {
    const res = await fetch(`${base}/v1/systemone`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: 'not-json{'
    });
    assert.equal(res.status, 400);
    const data = await res.json();
    assert.match(data.error, /Invalid JSON body/);
  });

  await t.test('non-object JSON body is a 400', async () => {
    const res = await fetch(`${base}/v1/systemone`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: 'null'
    });
    assert.equal(res.status, 400);
  });

  await t.test('valid body without a token is a 401', async () => {
    const res = await fetch(`${base}/v1/systemone`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ state: {}, questions: {} })
    });
    assert.equal(res.status, 401);
  });
});

test('clef-proxy upstream guard (#690/#692)', async (t) => {
  let stubBehavior = () => {};
  const stubPaths = [];
  const stub = http.createServer((req, res) => {
    stubPaths.push(req.url);
    stubBehavior(req, res);
  });
  const stubPort = await freePort();
  await new Promise((resolve) => stub.listen(stubPort, '127.0.0.1', resolve));
  t.after(() => { stub.close(); });

  const { child, base } = await startProxy({
    CLEF_PROXY_GATEWAY_BASE: `http://127.0.0.1:${stubPort}/v1`,
    CLEF_PROXY_DATA_PATH_TIMEOUT_MS: '300',
    NO_PROXY: '127.0.0.1,localhost',
    no_proxy: '127.0.0.1,localhost'
  });
  t.after(() => { child.kill('SIGTERM'); });

  const ask = () => fetch(`${base}/v1/systemone`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: 'Bearer test-token' },
    body: JSON.stringify({ state: {}, questions: { q1: 'question one' } })
  });

  await t.test('non-JSON upstream 502 stays a 502, not a 500', async () => {
    stubBehavior = (req, res) => {
      res.writeHead(502, { 'Content-Type': 'text/html' });
      res.end('<html>bad gateway</html>');
    };
    const res = await ask();
    assert.equal(res.status, 502);
    const data = await res.json();
    assert.match(data.error, /Cloudflare Workers AI Clef execution failed/);
  });

  await t.test('JSON null upstream 200 is a 502', async () => {
    stubBehavior = (req, res) => {
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end('null');
    };
    const res = await ask();
    assert.equal(res.status, 502);
  });

  await t.test('scalar upstream 200 is a 502', async () => {
    stubBehavior = (req, res) => {
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end('"just a string"');
    };
    const res = await ask();
    assert.equal(res.status, 502);
  });

  await t.test('happy path: stub success shape answers 200 with the answers', async () => {
    stubBehavior = (req, res) => {
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ success: true, result: { answers: { q1: 'answer one' } } }));
    };
    const res = await ask();
    assert.equal(res.status, 200);
    const data = await res.json();
    assert.deepEqual(data.answers, { q1: 'answer one' });
    assert.match(stubPaths.at(-1), /\/workers-ai\/run\//);
  });

  await t.test('upstream that never sends headers returns a 504', async () => {
    stubBehavior = () => {};
    const res = await ask();
    assert.equal(res.status, 504);
    const data = await res.json();
    assert.match(data.error, /timed out/i);
  });

  await t.test('response body that never completes returns a 504', async () => {
    stubBehavior = (_req, res) => {
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.write('{"success":true,"result":');
    };
    const res = await ask();
    assert.equal(res.status, 504);
    const data = await res.json();
    assert.match(data.error, /timed out/i);
  });
});

// The mtime credential cache (#768, contract per #782): config files
// are re-read only when their mtime changes. The token reaches the
// stub gateway as the Authorization header, so the stub sees exactly
// which token each request presented. No Authorization header is sent
// by the client here: the file is the only token source.
test('clef-proxy config cache contract (#782)', async (t) => {
  const seenAuth = [];
  const stub = http.createServer((req, res) => {
    seenAuth.push(req.headers.authorization || '');
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ success: true, result: { answers: { q1: 'ok' } } }));
  });
  const stubPort = await freePort();
  await new Promise((resolve) => stub.listen(stubPort, '127.0.0.1', resolve));
  t.after(() => { stub.close(); });

  const { child, base, tmp } = await startProxy({
    CLEF_PROXY_GATEWAY_BASE: `http://127.0.0.1:${stubPort}/v1`,
    NO_PROXY: '127.0.0.1,localhost',
    no_proxy: '127.0.0.1,localhost'
  });
  t.after(() => { child.kill('SIGTERM'); });

  const cfgDir = path.join(tmp, '.config', 'cloudflare');
  fs.mkdirSync(cfgDir, { recursive: true });
  const tokenFile = path.join(cfgDir, 'ai-inference-token');
  // Controlled mtimes (fs.utimesSync) — no reliance on wall-clock
  // mtime granularity when the file is rewritten.
  const T1 = new Date('2026-01-01T00:00:00.000Z');
  const T2 = new Date('2026-01-02T00:00:00.000Z');
  const writeToken = (value, mtime) => {
    fs.writeFileSync(tokenFile, value);
    fs.utimesSync(tokenFile, mtime, mtime);
  };
  const ask = () => fetch(`${base}/v1/systemone`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ state: {}, questions: { q1: 'question one' } })
  });

  await t.test('first request reads the token from the config file', async () => {
    writeToken('token-one', T1);
    const res = await ask();
    assert.equal(res.status, 200);
    assert.equal(seenAuth.at(-1), 'Bearer token-one');
  });

  await t.test('changed contents with an unchanged mtime are not re-read', async () => {
    writeToken('token-two', T1);
    const res = await ask();
    assert.equal(res.status, 200);
    // The cache serves the value keyed to T1: a re-read would have
    // presented token-two.
    assert.equal(seenAuth.at(-1), 'Bearer token-one');
  });

  await t.test('changed contents with a new mtime are re-read (rotation picked up)', async () => {
    writeToken('token-three', T2);
    const res = await ask();
    assert.equal(res.status, 200);
    assert.equal(seenAuth.at(-1), 'Bearer token-three');
  });

  await t.test('a missing token file yields the fallback (401, gateway not called)', async () => {
    fs.unlinkSync(tokenFile);
    const callsBefore = seenAuth.length;
    const res = await ask();
    assert.equal(res.status, 401);
    assert.equal(seenAuth.length, callsBefore);
  });
});

// Model grammar (#2410): the resolved model is interpolated raw into the
// gateway run URL, so anything outside the Clef family grammar
// (^@cf/cloudflare/clef[A-Za-z0-9._-]*$) must be refused with a 400
// BEFORE any upstream fetch, and Clef spellings must reach the stub at
// the exact expected run path.
test('clef-proxy model grammar (#2410)', async (t) => {
  const stubPaths = [];
  const stub = http.createServer((req, res) => {
    stubPaths.push(req.url);
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ success: true, result: { answers: { q1: 'answer one' } } }));
  });
  const stubPort = await freePort();
  await new Promise((resolve) => stub.listen(stubPort, '127.0.0.1', resolve));
  t.after(() => { stub.close(); });

  const { child, base } = await startProxy({
    CLEF_PROXY_GATEWAY_BASE: `http://127.0.0.1:${stubPort}/v1`,
    NO_PROXY: '127.0.0.1,localhost',
    no_proxy: '127.0.0.1,localhost'
  });
  t.after(() => { child.kill('SIGTERM'); });

  const ask = (model) => fetch(`${base}/v1/systemone`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: 'Bearer <redacted>' },
    body: JSON.stringify({ state: {}, questions: { q1: 'question one' }, ...(model === undefined ? {} : { model }) })
  });

  const hostile = [
    '@cf/meta/llama-3.1-8b-instruct',
    '@cf/cloudflare/../../ai/models/search',
    'clef/../../../../client/v4/accounts/OTHER/ai/run/x',
    '@cf/cloudflare/clef?per_page=1#frag',
    'my-clef-thing'
  ];
  for (const model of hostile) {
    await t.test(`hostile model is refused 400 with no upstream call: ${model}`, async () => {
      const callsBefore = stubPaths.length;
      const res = await ask(model);
      assert.equal(res.status, 400);
      const data = await res.json();
      assert.match(data.error, /Refused model/);
      assert.equal(stubPaths.length, callsBefore);
    });
  }

  const controls = [
    [undefined, '@cf/cloudflare/clef-flash'],
    ['clef', '@cf/cloudflare/clef'],
    ['clef-flash', '@cf/cloudflare/clef-flash'],
    ['@cf/cloudflare/clef', '@cf/cloudflare/clef'],
    ['@cf/cloudflare/clef-omni', '@cf/cloudflare/clef-omni']
  ];
  for (const [model, resolved] of controls) {
    await t.test(`control model reaches the stub at the exact run path: ${model}`, async () => {
      const res = await ask(model);
      assert.equal(res.status, 200);
      assert.ok(stubPaths.at(-1).endsWith(`/workers-ai/run/${resolved}`), stubPaths.at(-1));
    });
  }
});
