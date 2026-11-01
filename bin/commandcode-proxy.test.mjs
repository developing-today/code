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
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const binDir = path.dirname(fileURLToPath(import.meta.url));

test('commandcode-proxy rejects invalid timeout values at startup', () => {
  for (const raw of ['abc', '240000junk', '0', '-1', '2147483648']) {
    const result = spawnSync(process.execPath, [path.join(binDir, 'commandcode-proxy.mjs')], {
      env: {
        ...process.env,
        COMMANDCODE_PROXY_PORT: '18959',
        COMMANDCODE_PROXY_HOST: '127.0.0.1',
        COMMANDCODE_PROXY_TIMEOUT_MS: raw,
      },
      encoding: 'utf8',
      timeout: 10000,
    });
    assert.equal(result.status, 1, `value ${raw}: ${result.stderr}`);
    assert.match(result.stderr, /COMMANDCODE_PROXY_TIMEOUT_MS/);
    assert.ok(result.stderr.includes(`"${raw}"`), result.stderr);
    assert.doesNotMatch(result.stdout, /listening/);
  }
});

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

// ---------------------------------------------------------------------------
// ACP-speaking stub (#3114): answers the ACP handshake for real so the proxy
// stays on the ACP path, and returns prompt usage carrying cacheReadTokens
// (omitted when the prompt contains NOCACHE). Spawn (-p) invocations keep
// the canned NDJSON behaviour.
// ---------------------------------------------------------------------------
const ACP_STUB = `#!/usr/bin/env node
const args = process.argv.slice(2);
if (args[0] !== 'acp') {
  let stdin = '';
  process.stdin.on('data', (c) => { stdin += c; });
  process.stdin.on('end', () => {
    process.stdout.write(JSON.stringify({ type: 'event', event: { type: 'text_delta', delta: 'STUB-OK' } }) + '\\n');
    process.stdout.write(JSON.stringify({ type: 'event', event: { type: 'run_end', result: { finalText: 'STUB-OK', usage: { inputTokens: 10, outputTokens: 2 } } } }) + '\\n');
    process.exit(0);
  });
  return;
}
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
    if (m.id === undefined || !m.method) continue;
    const reply = (result) => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: m.id, result }) + '\\n');
    if (m.method === 'initialize') {
      reply({ protocolVersion: 1, agentInfo: { name: 'stub-acp', version: '0' }, agentCapabilities: { loadSession: false } });
    } else if (m.method === 'session/new') {
      reply({ sessionId: 'stub-acp-session-1' });
    } else if (m.method === 'session/prompt') {
      const sessionId = m.params.sessionId;
      const text = (m.params.prompt && m.params.prompt[0] && m.params.prompt[0].text) || '';
      process.stdout.write(JSON.stringify({ jsonrpc: '2.0', method: 'session/update', params: { sessionId, update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'ACP-OK' } } } }) + '\\n');
      const usage = text.includes('NOCACHE')
        ? { inputTokens: 50, outputTokens: 4 }
        : { inputTokens: 50, outputTokens: 4, cacheReadTokens: 77 };
      reply({ stopReason: 'end_turn', usage });
    } else {
      reply({});
    }
  }
});
`;

test('ACP prompt usage cacheReadTokens surfaces as prompt_tokens_details.cached_tokens (#3114)', async (t) => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'cc-proxy-acp-test-'));
  const stubHome = path.join(tmp, 'home');
  const stubBin = path.join(stubHome, 'tools', 'command-code', 'bin');
  fs.mkdirSync(stubBin, { recursive: true });
  fs.writeFileSync(path.join(stubBin, 'commandcode'), ACP_STUB, { mode: 0o755 });

  const port = 18949;
  const env = {
    ...process.env,
    HOME: stubHome,
    COMMANDCODE_PROXY_PORT: String(port),
    COMMANDCODE_PROXY_HOST: '127.0.0.1',
    COMMANDCODE_PROXY_EGRESS: '',
    COMMANDCODE_PROXY_TIMEOUT_MS: '30000',
    COMMANDCODE_PROXY_WORKDIR: path.join(tmp, 'work'),
  };
  const child = spawn('node', [path.join(binDir, 'commandcode-proxy.mjs')], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  let logged = '';
  child.stdout.on('data', (c) => { logged += c; });
  child.stderr.on('data', (c) => { logged += c; });
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
    return text;
  };

  // Non-streaming: ACP usage cacheReadTokens 77 must surface as cached_tokens.
  const plain = JSON.parse(await chat({ model: 'deepseek/deepseek-v4-flash', messages: [{ role: 'user', content: 'hi cached' }], stream: false }));
  assert.equal(plain.choices[0].message.content, 'ACP-OK');
  assert.equal(plain.usage.prompt_tokens_details.cached_tokens, 77, 'ACP cacheReadTokens must surface as prompt_tokens_details.cached_tokens (opencode #394)');

  // Streaming: the end chunk carries the same cached_tokens.
  const sse = await chat({ model: 'deepseek/deepseek-v4-flash', messages: [{ role: 'user', content: 'hi cached stream' }], stream: true });
  const events = sse.split('\n').filter(l => l.startsWith('data: ') && !l.includes('[DONE]')).map(l => JSON.parse(l.slice('data: '.length)));
  const endChunk = events.find(e => e.usage);
  assert.ok(endChunk, 'stream end chunk must carry usage');
  assert.equal(endChunk.usage.prompt_tokens_details.cached_tokens, 77, 'streaming ACP path must surface cached_tokens');

  // Zero/absent cache value: prompt_tokens_details omitted entirely.
  const noCache = JSON.parse(await chat({ model: 'deepseek/deepseek-v4-flash', messages: [{ role: 'user', content: 'NOCACHE please' }], stream: false }));
  assert.equal(noCache.choices[0].message.content, 'ACP-OK');
  assert.equal(noCache.usage.prompt_tokens_details, undefined, 'absent cacheReadTokens must omit prompt_tokens_details entirely');
});

test('non-object JSON body is a 400 and the proxy stays alive (#2816)', async (t) => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'cc-proxy-nullbody-'));
  const stubHome = path.join(tmp, 'home');
  const stubBin = path.join(stubHome, 'tools', 'command-code', 'bin');
  fs.mkdirSync(stubBin, { recursive: true });
  fs.writeFileSync(path.join(stubBin, 'commandcode'), STUB, { mode: 0o755 });
  const port = 18953;
  const env = {
    ...process.env,
    HOME: stubHome,
    COMMANDCODE_PROXY_PORT: String(port),
    COMMANDCODE_PROXY_HOST: '127.0.0.1',
    COMMANDCODE_PROXY_EGRESS: '',
    COMMANDCODE_PROXY_TIMEOUT_MS: '30000',
    STUB_RECORD_DIR: path.join(tmp, 'rec'),
  };
  fs.mkdirSync(path.join(tmp, 'rec'), { recursive: true });
  const child = spawn('node', [path.join(binDir, 'commandcode-proxy.mjs')], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  t.after(() => { try { child.kill('SIGKILL'); } catch {} });
  await waitFor(`http://127.0.0.1:${port}/health`);
  for (const bad of ['null', '[1,2]', '"x"', '42']) {
    const res = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: bad
    });
    assert.equal(res.status, 400, `body ${bad} must be a 400, got ${res.status}`);
    await res.text();
  }
  const alive = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{bad json'
  });
  assert.equal(alive.status, 400, 'proxy must still serve after non-object bodies');
  await alive.text();
});
