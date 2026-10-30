#!/usr/bin/env node
// Tests for commandcode-proxy's chat/agency split.
// Run: node --test bin/commandcode-proxy.test.mjs
//
// A stub `commandcode` (preferred binary path under a temp HOME) fails the
// ACP handshake fast (so the proxy falls through to the spawn path) and
// answers -p runs with canned NDJSON while recording argv+stdin. The tests
// assert:
// - chat requests (no tools) spawn single-turn `--permission-mode dont-ask`
//   with the no-tools steer and never `--tools-all`;
// - agency requests (tools present) spawn `--tools-all`, a multi-turn
//   budget, no `dont-ask`, with the workdir steer.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const binDir = path.dirname(fileURLToPath(import.meta.url));

const STUB = `#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');
const rec = process.env.STUB_RECORD_DIR;
const args = process.argv.slice(2);
if (args[0] === 'acp') {
  // Fail the ACP handshake fast: answer initialize with an error so the
  // proxy falls back to the spawn path without waiting out its timeout.
  let buf = '';
  process.stdin.on('data', (c) => {
    buf += c.toString('utf8');
    const lines = buf.split('\\n');
    buf = lines.pop();
    for (const line of lines) {
      const t = line.trim();
      if (!t || !t.startsWith('{')) continue;
      let m;
      try { m = JSON.parse(t); } catch { continue; }
      if (m.method === 'initialize') {
        process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: m.id, error: { code: -32000, message: 'stub: no acp' } }) + '\\n');
        process.exit(0);
      }
    }
  });
  return;
}
let stdin = '';
process.stdin.on('data', (c) => { stdin += c; });
process.stdin.on('end', () => {
  fs.mkdirSync(rec, { recursive: true });
  fs.writeFileSync(path.join(rec, 'argv.json'), JSON.stringify(args));
  fs.writeFileSync(path.join(rec, 'stdin.txt'), stdin);
  process.stdout.write(JSON.stringify({ type: 'event', event: { type: 'text_delta', delta: 'STUB-OK' } }) + '\\n');
  process.stdout.write(JSON.stringify({ type: 'event', event: { type: 'run_end', result: { finalText: 'STUB-OK', usage: { inputTokens: 10, outputTokens: 2 } } } }) + '\\n');
  process.exit(0);
});
`;

const waitFor = async (url, timeoutMs = 15000) => {
  const start = Date.now();
  for (;;) {
    try {
      const res = await fetch(url);
      if (res.ok) return;
    } catch {}
    if (Date.now() - start > timeoutMs) throw new Error(`timed out waiting for ${url}`);
    await new Promise((r) => setTimeout(r, 200));
  }
};

test('chat vs agency spawn contract', async (t) => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'cc-proxy-test-'));
  const stubHome = path.join(tmp, 'home');
  const stubBin = path.join(stubHome, 'tools', 'command-code', 'bin');
  fs.mkdirSync(stubBin, { recursive: true });
  fs.writeFileSync(path.join(stubBin, 'commandcode'), STUB, { mode: 0o755 });
  const recDir = path.join(tmp, 'rec');
  fs.mkdirSync(recDir, { recursive: true });

  const port = 18948;
  const env = {
    ...process.env,
    HOME: stubHome,
    COMMANDCODE_PROXY_PORT: String(port),
    COMMANDCODE_PROXY_HOST: '127.0.0.1',
    COMMANDCODE_PROXY_EGRESS: '',
    COMMANDCODE_PROXY_TIMEOUT_MS: '30000',
    STUB_RECORD_DIR: recDir,
  };
  const child = spawn('node', [path.join(binDir, 'commandcode-proxy.mjs')], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  let logged = '';
  child.stdout.on('data', (c) => { logged += c; });
  child.stderr.on('data', (c) => { logged += c; });
  child.on('exit', (code, signal) => { logged += `\n[PROXY EXIT code=${code} signal=${signal}]`; });
  t.after(() => { try { child.kill('SIGKILL'); } catch {} });
  t.after(() => { if (process.env.CC_TEST_DEBUG) console.log(logged); });

  await waitFor(`http://127.0.0.1:${port}/health`);

  const chat = async (body) => {
    const res = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    const text = await res.text();
    assert.equal(res.status, 200, `expected 200, got ${res.status}: ${text.slice(0, 500)}`);
    return JSON.parse(text);
  };

  // Chat: no tools (system + user, as OpenCode sends).
  const chatRes = await chat({ model: 'deepseek/deepseek-v4-flash', messages: [{ role: 'system', content: 'sys' }, { role: 'user', content: 'hi' }], stream: false });
  assert.equal(chatRes.choices[0].message.content, 'STUB-OK');
  let argv = JSON.parse(fs.readFileSync(path.join(recDir, 'argv.json'), 'utf8'));
  let stdin = fs.readFileSync(path.join(recDir, 'stdin.txt'), 'utf8');
  assert.ok(argv.includes('dont-ask'), `chat must pass dont-ask: ${argv.join(' ')}`);
  assert.ok(!argv.includes('--tools-all'), `chat must not pass --tools-all: ${argv.join(' ')}`);
  const mt = argv[argv.indexOf('--max-turns') + 1];
  assert.equal(mt, '1', `chat must be single-turn, got max-turns=${mt}`);
  assert.ok(stdin.includes('Do not invoke tools'), 'chat prompt must carry the no-tools steer');

  // Agency: tools present.
  const agencyRes = await chat({
    model: 'deepseek/deepseek-v4-flash',
    messages: [{ role: 'user', content: 'write it' }],
    tools: [{ type: 'function', function: { name: 'write_file', description: 'write a file' } }],
    stream: false,
  });
  assert.equal(agencyRes.choices[0].message.content, 'STUB-OK');
  argv = JSON.parse(fs.readFileSync(path.join(recDir, 'argv.json'), 'utf8'));
  stdin = fs.readFileSync(path.join(recDir, 'stdin.txt'), 'utf8');
  assert.ok(argv.includes('--tools-all'), `agency must pass --tools-all: ${argv.join(' ')}`);
  assert.ok(!argv.includes('dont-ask'), `agency must not pass dont-ask: ${argv.join(' ')}`);
  const amt = argv[argv.indexOf('--max-turns') + 1];
  assert.ok(Number(amt) > 1, `agency must allow multiple turns, got max-turns=${amt}`);
  assert.ok(stdin.includes('Do ALL file work inside'), 'agency prompt must carry the workdir steer');
  assert.ok(stdin.includes('write_file'), 'agency prompt must name the requested tool');
});
