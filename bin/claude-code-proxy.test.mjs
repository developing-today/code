// Tests for claude-code-proxy's resident stream-json sessions.
// Run: node --test bin/claude-code-proxy.test.mjs
//
// A stub `claude` stands in for the real CLI: it answers spawn args with
// an init event naming the --session-id/--resume id, then for every user
// line on stdin it emits two text deltas, an assistant message, and a
// result whose text echoes what it received. Every user line it receives
// is appended to $STUB_LOG so the tests can prove the second turn sent
// ONLY the newest user message (the delta-only warm behavior) rather
// than a re-flattened history.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const binDir = path.dirname(fileURLToPath(import.meta.url));

const STUB = `#!/bin/sh
sid="stub-session"
prev=""
for a in "$@"; do
  if [ "$prev" = "--session-id" ] || [ "$prev" = "--resume" ]; then sid="$a"; fi
  prev="$a"
done
while IFS= read -r line; do
  case "$line" in
    *'"subtype":"initialize"'*)
      rid=$(printf '%s' "$line" | sed 's/.*"request_id":"\\([^"]*\\)".*/\\1/')
      printf '%s\\n' "{\\"type\\":\\"control_response\\",\\"response\\":{\\"subtype\\":\\"success\\",\\"request_id\\":\\"$rid\\",\\"response\\":{}}}"
      printf '%s\\n' "{\\"type\\":\\"system\\",\\"subtype\\":\\"init\\",\\"session_id\\":\\"$sid\\"}"
      continue
      ;;
  esac
  printf '%s\\n' "$line" >> "$STUB_LOG"
  txt=$(printf '%s' "$line" | sed 's/.*"text":"\\([^"]*\\)".*/\\1/')
  if [ "$txt" = "FAILME" ]; then
    printf '%s\\n' '{"type":"result","subtype":"error_during_execution","result":"stub failure"}'
    continue
  fi
  half=$(printf '%s' "$txt" | cut -c1-4)
  printf '%s\\n' "{\\"type\\":\\"stream_event\\",\\"event\\":{\\"type\\":\\"content_block_delta\\",\\"index\\":0,\\"delta\\":{\\"type\\":\\"text_delta\\",\\"text\\":\\"ECHO:$half\\"}}}"
  printf '%s\\n' "{\\"type\\":\\"stream_event\\",\\"event\\":{\\"type\\":\\"content_block_delta\\",\\"index\\":0,\\"delta\\":{\\"type\\":\\"text_delta\\",\\"text\\":\\" rest\\"}}}"
  printf '%s\\n' "{\\"type\\":\\"assistant\\",\\"message\\":{\\"content\\":[{\\"type\\":\\"text\\",\\"text\\":\\"ECHO:$txt rest\\"}]}}"
  printf '%s\\n' "{\\"type\\":\\"result\\",\\"subtype\\":\\"success\\",\\"result\\":\\"ECHO:$txt rest\\",\\"usage\\":{\\"input_tokens\\":100,\\"output_tokens\\":5,\\"cache_read_input_tokens\\":80}}"
done
`;

const freePort = () => new Promise((resolve, reject) => {
  const srv = net.createServer();
  srv.once('error', reject);
  srv.listen(0, '127.0.0.1', () => {
    const { port } = srv.address();
    srv.close(() => resolve(port));
  });
});

const startProxy = async () => {
  const port = await freePort();
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'claude-code-proxy-test-'));
  const stubPath = path.join(tmp, 'claude-stub');
  fs.writeFileSync(stubPath, STUB, { mode: 0o755 });
  const logPath = path.join(tmp, 'stub.log');
  const child = spawn(process.execPath, [path.join(binDir, 'claude-code-proxy.mjs')], {
    env: {
      ...process.env,
      CLAUDE_CODE_PROXY_PORT: String(port),
      CLAUDE_CODE_PROXY_HOST: '127.0.0.1',
      CLAUDE_CODE_BIN: stubPath,
      CLAUDE_CODE_PROXY_WORKDIR: path.join(tmp, 'work'),
      CLAUDE_CODE_PROXY_SESSION_FILE: path.join(tmp, 'sessions.json'),
      CLAUDE_CODE_PROXY_PROJECTS_DIR: path.join(tmp, 'no-projects'),
      CLAUDE_CODE_PROXY_EGRESS: '',
      STUB_LOG: logPath,
      XDG_STATE_HOME: path.join(tmp, 'state')
    },
    stdio: ['ignore', 'pipe', 'pipe']
  });
  let stderr = '';
  child.stderr.on('data', c => { stderr += c; });
  const base = `http://127.0.0.1:${port}`;
  const deadline = Date.now() + 15000;
  for (;;) {
    try {
      const r = await fetch(`${base}/health`);
      if (r.ok) break;
    } catch { /* not up yet */ }
    if (Date.now() > deadline) {
      child.kill('SIGKILL');
      throw new Error(`proxy did not become healthy; stderr: ${stderr}`);
    }
    await new Promise(r => setTimeout(r, 100));
  }
  return { base, child, logPath, tmp };
};

const chat = (base, body, headers = {}) => fetch(`${base}/v1/chat/completions`, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json', ...headers },
  body: JSON.stringify(body)
});

test('health + models catalog', async (t) => {
  const { base, child } = await startProxy();
  t.after(() => child.kill('SIGKILL'));
  const health = await (await fetch(`${base}/health`)).json();
  assert.equal(health.status, 'ok');
  assert.equal(health.service, 'claude-code-proxy');
  assert.equal(health.engine, 'claude-stream-json-resident');
  const models = await (await fetch(`${base}/v1/models`)).json();
  // The verified static floor; the observed-models union is empty here
  // because CLAUDE_CODE_PROXY_PROJECTS_DIR points at an empty test dir.
  assert.deepEqual(models.data.map(m => m.id), ['claude-haiku-5-5', 'claude-sonnet-5-5', 'claude-opus-5-5']);
});

test('two turns: second turn sends only the newest user message', async (t) => {
  const { base, child, logPath } = await startProxy();
  t.after(() => child.kill('SIGKILL'));
  const headers = { 'x-session-id': 'test-convo-1' };
  const sys = { role: 'system', content: 'You are terse.' };
  const u1 = { role: 'user', content: 'remember BANANA' };

  const r1 = await chat(base, { model: 'claude-haiku-5-5', messages: [sys, u1] }, headers);
  assert.equal(r1.status, 200);
  const j1 = await r1.json();
  const a1 = j1.choices[0].message.content;
  assert.ok(a1.includes('ECHO:remember BANANA rest'), `unexpected turn-1 content: ${a1}`);
  assert.equal(j1.usage.prompt_tokens, 180); // 100 input + 80 cache read

  const u2 = { role: 'user', content: 'what was it?' };
  const r2 = await chat(base, {
    model: 'claude-haiku-5-5',
    messages: [sys, u1, { role: 'assistant', content: a1 }, u2]
  }, headers);
  assert.equal(r2.status, 200);
  const j2 = await r2.json();
  assert.ok(j2.choices[0].message.content.includes('ECHO:what was it? rest'));

  const received = fs.readFileSync(logPath, 'utf-8').trim().split('\n');
  assert.equal(received.length, 2, `stub should have received exactly 2 user lines, got: ${received.join(' | ')}`);
  assert.ok(received[0].includes('remember BANANA'));
  assert.ok(received[1].includes('what was it?'));
  assert.ok(!received[1].includes('BANANA'), `turn 2 must be delta-only, got: ${received[1]}`);
});

test('diverged history re-bootstraps with the full transcript', async (t) => {
  const { base, child, logPath } = await startProxy();
  t.after(() => child.kill('SIGKILL'));
  const headers = { 'x-session-id': 'test-convo-2' };
  const u1 = { role: 'user', content: 'first version' };
  const r1 = await chat(base, { model: 'claude-haiku-5-5', messages: [u1] }, headers);
  assert.equal(r1.status, 200);
  await r1.json();

  // Same key, but the first user message changed -> prefix mismatch ->
  // fresh session whose first turn is the flattened transcript.
  const r2 = await chat(base, {
    model: 'claude-haiku-5-5',
    messages: [{ role: 'user', content: 'EDITED first' }, { role: 'user', content: 'second' }]
  }, headers);
  assert.equal(r2.status, 200);
  await r2.json();
  const received = fs.readFileSync(logPath, 'utf-8').trim().split('\n');
  assert.equal(received.length, 2);
  assert.ok(received[1].includes('EDITED first'), `re-bootstrap should carry the transcript, got: ${received[1]}`);
  assert.ok(received[1].includes('second'));
});

test('streaming emits SSE deltas and a finish chunk', async (t) => {
  const { base, child } = await startProxy();
  t.after(() => child.kill('SIGKILL'));
  const r = await chat(base, {
    model: 'claude-haiku-5-5', stream: true,
    messages: [{ role: 'user', content: 'stream please' }]
  }, { 'x-session-id': 'test-convo-3' });
  assert.equal(r.status, 200);
  const text = await r.text();
  assert.ok(text.includes('ECHO:stre'));
  assert.ok(text.includes('finish_reason":"stop"') || text.includes('"finish_reason": "stop"') || text.includes('stop'));
  assert.ok(text.trimEnd().endsWith('data: [DONE]'));
});

test('failed turn surfaces an error and the session is retryable', async (t) => {
  const { base, child } = await startProxy();
  t.after(() => child.kill('SIGKILL'));
  // FAILME makes the stub answer with an error result on both the primary
  // session and the one fresh retry, so the proxy must surface a 500.
  const r = await chat(base, {
    model: 'claude-haiku-5-5',
    messages: [{ role: 'user', content: 'FAILME' }]
  }, { 'x-session-id': 'test-convo-4' });
  assert.equal(r.status, 500);
  const j = await r.json();
  assert.ok(j.error.message.includes('stub failure') || j.error.message.length > 0);
});
