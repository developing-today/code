// Tests for clef-proxy's request-error handling (#690).
// Run: node --test bin/clef-proxy.test.mjs
//
// The proxy forwards valid requests to the Cloudflare AI gateway, which
// these tests never reach: they cover the paths that answer locally —
// health, a malformed client body (400, not the pre-fix 500 from the
// shared upstream try/catch), and a missing inference token (401, with
// HOME pointed at an empty dir and no token in the environment).
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
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

const startProxy = async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'clef-proxy-test-'));
  const port = await freePort();
  const env = { ...process.env, HOME: tmp, CLEF_PROXY_PORT: String(port), XDG_STATE_HOME: path.join(tmp, 'state') };
  delete env.CLOUDFLARE_API_TOKEN;
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
  return { child, base: `http://127.0.0.1:${port}` };
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
