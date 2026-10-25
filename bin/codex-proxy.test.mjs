// Tests for codex-proxy's failure detection (#676, #680).
// Run: node --test bin/codex-proxy.test.mjs
//
// The proxy spawns one `codex exec` child per request, with the prompt
// delivered on stdin. These tests put a stub `codex` at
// $HOME/.local/bin/codex (the proxy's binary resolution prefers it)
// whose behaviour is keyed on markers in the prompt: a run that exits
// non-zero used to be reported as a successful 'stop' — empty when it
// produced nothing, and carrying the truncated text when it produced
// partial output.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const binDir = path.dirname(fileURLToPath(import.meta.url));

const STUB = `#!/bin/sh
prompt="$(cat)"
resume=0
for a in "$@"; do [ "$a" = "resume" ] && resume=1; done
{
  echo "ARGS:$*"
  echo "RESUME:$resume"
  echo "PROMPT:$prompt"
  echo "==="
} >> "$HOME/invocations.log"
if [ "$resume" = "1" ]; then
  case "$prompt" in
    *RESUMEFAIL*)
      echo "stub codex: session not found" >&2
      exit 3 ;;
    *)
      printf '%s\\n' '{"type":"thread.started","thread_id":"stub-session-1"}'
      printf '%s\\n' '{"type":"item.completed","item":{"type":"agent_message","text":"resumed answer"}}'
      printf '%s\\n' '{"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":3}}'
      exit 0 ;;
  esac
fi
case "$prompt" in
  *FAILME*)
    echo "stub codex: auth expired" >&2
    exit 3 ;;
  *FAILPARTIAL*)
    printf '%s\\n' '{"type":"item.completed","item":{"type":"agent_message","text":"partial answer"}}'
    echo "stub codex: run failed midway" >&2
    exit 3 ;;
  *INBANDPARTIAL*)
    printf '%s\\n' '{"type":"item.completed","item":{"type":"agent_message","text":"partial answer"}}'
    printf '%s\\n' '{"type":"turn.failed","error":{"message":"stub codex: run failed midway"}}'
    exit 0 ;;
  *INBANDFAIL*)
    printf '%s\\n' '{"type":"turn.failed","error":{"message":"stub codex: model overloaded"}}'
    exit 0 ;;
  *SIGKILLME*)
    kill -9 $$ ;;
  *)
    printf '%s\\n' '{"type":"thread.started","thread_id":"stub-session-1"}'
    printf '%s\\n' '{"type":"item.completed","item":{"type":"agent_message","text":"Hello world"}}'
    printf '%s\\n' '{"type":"turn.completed","usage":{"input_tokens":100,"output_tokens":3}}'
    exit 0 ;;
esac
`;


// ---------------------------------------------------------------------------
// Fake app-server daemon (#480): a real WebSocket-over-Unix-socket server
// speaking just enough of the codex app-server protocol for the proxy's
// transport tests — initialize, thread/start, thread/resume, turn/start,
// agentMessage deltas, token usage, turn/completed. Behaviour is keyed on
// markers in the turn text, mirroring the exec stub.
// ---------------------------------------------------------------------------
const wsServerFrame = (text) => {
  const data = Buffer.from(text);
  let header;
  if (data.length < 126) {
    header = Buffer.from([0x81, data.length]);
  } else {
    header = Buffer.alloc(4);
    header[0] = 0x81; header[1] = 126; header.writeUInt16BE(data.length, 2);
  }
  return Buffer.concat([header, data]);
};

const createFakeAppServer = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'codex-as-test-'));
  const socketPath = path.join(dir, 'app-server-control.sock');
  const requests = [];
  const sockets = new Set();
  const server = net.createServer((sock) => {
    sockets.add(sock);
    sock.on('close', () => sockets.delete(sock));
    let buf = Buffer.alloc(0);
    let headersDone = false;
    let frags = [];
    const sendObj = (obj) => sock.write(wsServerFrame(JSON.stringify(obj)));
    const handle = (msg) => {
      if (msg.method === 'initialize') {
        requests.push({ method: 'initialize' });
        sendObj({ id: msg.id, result: { userAgent: 'stub-app-server', codexHome: dir, platformFamily: 'unix', platformOs: 'linux' } });
      } else if (msg.method === 'initialized') {
        requests.push({ method: 'initialized' });
      } else if (msg.method === 'thread/start') {
        requests.push({ method: 'thread/start', params: msg.params });
        sendObj({ id: msg.id, result: { thread: { id: 'stub-thread-1' }, model: 'gpt-5.6-luna' } });
      } else if (msg.method === 'thread/resume') {
        requests.push({ method: 'thread/resume', params: msg.params });
        sendObj({ id: msg.id, result: { thread: { id: msg.params.threadId } } });
      } else if (msg.method === 'turn/start') {
        const text = msg.params.input.map(i => i.text || '').join('\n');
        requests.push({ method: 'turn/start', params: msg.params, text });
        if (text.includes('ASRPCFAIL')) {
          sendObj({ id: msg.id, error: { code: -32000, message: 'stub app-server: rpc exploded' } });
          return;
        }
        sendObj({ id: msg.id, result: { turn: { id: 'stub-turn-1' } } });
        if (text.includes('ASKILL')) {
          // Daemon dies mid-turn: destroy the connection with no completion.
          setTimeout(() => { for (const s2 of sockets) s2.destroy(); }, 20);
          return;
        }
        const tid = msg.params.threadId;
        if (text.includes('ASFAIL')) {
          sendObj({ method: 'turn/completed', params: { threadId: tid, turn: { id: 'stub-turn-1', status: 'failed', error: { message: 'stub app-server: turn exploded' } } } });
          return;
        }
        sendObj({ method: 'item/agentMessage/delta', params: { threadId: tid, turnId: 'stub-turn-1', itemId: 'item-1', delta: 'app-' } });
        sendObj({ method: 'item/agentMessage/delta', params: { threadId: tid, turnId: 'stub-turn-1', itemId: 'item-1', delta: 'server answer' } });
        sendObj({ method: 'thread/tokenUsage/updated', params: { threadId: tid, turnId: 'stub-turn-1', tokenUsage: { last: { inputTokens: 42, outputTokens: 5 }, total: { inputTokens: 42, outputTokens: 5 } } } });
        sendObj({ method: 'turn/completed', params: { threadId: tid, turn: { id: 'stub-turn-1', status: 'completed' } } });
      }
    };
    sock.on('data', (chunk) => {
      buf = Buffer.concat([buf, chunk]);
      if (!headersDone) {
        const idx = buf.indexOf('\r\n\r\n');
        if (idx === -1) return;
        const head = buf.subarray(0, idx).toString('utf-8');
        buf = buf.subarray(idx + 4);
        const key = /Sec-WebSocket-Key: (\S+)/i.exec(head)[1];
        const accept = crypto.createHash('sha1').update(key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
        sock.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
        headersDone = true;
      }
      for (;;) {
        if (buf.length < 2) return;
        const fin = (buf[0] & 0x80) !== 0;
        const opcode = buf[0] & 0x0f;
        let len = buf[1] & 0x7f;
        let off = 2;
        if (len === 126) { if (buf.length < 4) return; len = buf.readUInt16BE(2); off = 4; }
        else if (len === 127) { if (buf.length < 10) return; len = Number(buf.readBigUInt64BE(2)); off = 10; }
        const masked = (buf[1] & 0x80) !== 0;
        const mask = masked ? buf.subarray(off, off + 4) : null;
        if (masked) off += 4;
        if (buf.length < off + len) return;
        let payload = buf.subarray(off, off + len);
        buf = buf.subarray(off + len);
        if (masked) {
          payload = Buffer.from(payload);
          for (let i = 0; i < payload.length; i++) payload[i] ^= mask[i % 4];
        }
        if (opcode === 0x8) { sock.end(); return; }
        if (opcode === 0x9 || opcode === 0xA) continue;
        frags.push(payload);
        if (!fin) continue;
        const text = Buffer.concat(frags).toString('utf-8');
        frags = [];
        try { handle(JSON.parse(text)); } catch { /* ignore */ }
      }
    });
  });
  return new Promise((resolve) => {
    server.listen(socketPath, () => resolve({
      socketPath,
      requests,
      close: () => { for (const s2 of sockets) s2.destroy(); server.close(); }
    }));
  });
};

const freePort = () => new Promise((resolve, reject) => {
  const srv = net.createServer();
  srv.once('error', reject);
  srv.listen(0, '127.0.0.1', () => {
    const { port } = srv.address();
    srv.close(() => resolve(port));
  });
});

const startProxy = async (reuse, extraEnv = {}) => {
  const tmp = reuse ? reuse.tmp : fs.mkdtempSync(path.join(os.tmpdir(), 'codex-proxy-test-'));
  const home = path.join(tmp, 'home');
  fs.mkdirSync(path.join(home, '.local', 'bin'), { recursive: true });
  fs.writeFileSync(path.join(home, '.local', 'bin', 'codex'), STUB, { mode: 0o755 });
  const stateHome = path.join(tmp, 'state');
  const port = await freePort();
  const child = spawn(process.execPath, [path.join(binDir, 'codex-proxy.mjs')], {
    env: {
      ...process.env,
      HOME: home,
      XDG_STATE_HOME: stateHome,
      CODEX_PROXY_PORT: String(port),
      ...extraEnv
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
  return { port, tmp, home, stateHome, stop: () => child.kill('SIGKILL') };
};

const post = (port, body) => fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(body)
});

const assertFailure = async (port, marker, detail) => {
  const messages = [{ role: 'user', content: `please ${marker} now` }];

  const plainRes = await post(port, { messages });
  assert.equal(plainRes.status, 500, `${marker}: non-stream must be a 500, not a 200 'stop'`);
  const plainBody = JSON.parse(await plainRes.text());
  assert.match(plainBody.error.message, detail, `${marker}: non-stream detail must name the cause, got: ${plainBody.error.message}`);
  assert.doesNotMatch(plainBody.error.message, /code (null|-2)\b/);

  const streamRes = await post(port, { messages, stream: true });
  assert.equal(streamRes.status, 200);
  assert.match(streamRes.headers.get('content-type'), /text\/event-stream/);
  const sse = await streamRes.text();
  const dataLines = sse.split('\n').filter(l => l.startsWith('data: '));
  assert.equal(dataLines[dataLines.length - 1], 'data: [DONE]', `${marker}: stream must end with [DONE]`);
  const events = dataLines.slice(0, -1).map(l => JSON.parse(l.slice('data: '.length)));
  const errorEvent = events.find(e => e.error);
  assert.ok(errorEvent, `${marker}: stream must carry an error event, got: ${JSON.stringify(sse)}`);
  assert.match(errorEvent.error.message, detail, `${marker}: stream detail must name the cause, got: ${errorEvent.error.message}`);
  const stopChunk = events.find(e => e.choices && e.choices[0].finish_reason === 'stop');
  assert.equal(stopChunk, undefined, `${marker}: no chunk may claim finish_reason 'stop'`);
};

test('successful run is unchanged in both modes', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  const messages = [{ role: 'user', content: 'hi' }];

  const plainRes = await post(port, { messages });
  assert.equal(plainRes.status, 200);
  const plain = JSON.parse(await plainRes.text());
  assert.equal(plain.choices[0].message.content, 'Hello world');
  assert.equal(plain.choices[0].finish_reason, 'stop');

  const streamRes = await post(port, { messages, stream: true });
  assert.equal(streamRes.status, 200);
  const sse = await streamRes.text();
  assert.match(sse, /Hello world/);
  assert.match(sse, /"finish_reason":"stop"/);
  assert.ok(sse.trimEnd().endsWith('data: [DONE]'));
});

test('execution failure with no output: 500 non-stream, SSE error + [DONE] stream (#676)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  await assertFailure(port, 'FAILME', /Codex execution failed: stub codex: auth expired/);
});

test('partial output then failure: masked success becomes an error in both modes (#676)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  await assertFailure(port, 'FAILPARTIAL', /Codex execution failed: stub codex: run failed midway/);
});

test('turn.failed with exit 0 and no output: 500 non-stream, SSE error + [DONE] stream (#680)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  await assertFailure(port, 'INBANDFAIL', /Codex execution failed: stub codex: model overloaded/);
});

test('partial output then turn.failed with exit 0: masked success becomes an error in both modes (#680)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  await assertFailure(port, 'INBANDPARTIAL', /Codex execution failed: stub codex: run failed midway/);
});

test('empty-stderr fallback names the killing signal, not code null (#676)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  await assertFailure(port, 'SIGKILLME', /Codex execution failed: codex exec terminated by signal SIGKILL/);
});

const readInvocations = (home) => {
  const raw = fs.readFileSync(path.join(home, 'invocations.log'), 'utf-8');
  return raw
    .split('===\n')
    .filter(s => s.trim())
    .map(block => {
      const resume = /RESUME:(\d)/.exec(block);
      const prompt = /PROMPT:([\s\S]*)$/.exec(block);
      return { resume: resume && resume[1] === '1', prompt: prompt ? prompt[1] : '' };
    });
};

const TURN1 = [{ role: 'user', content: 'alpha first question' }];
const TURN2 = [
  { role: 'user', content: 'alpha first question' },
  { role: 'assistant', content: 'Hello world' },
  { role: 'user', content: 'beta second question' }
];

test('second turn resumes the warm session with only the new tail (#480)', async (t) => {
  const { port, home, stateHome, stop } = await startProxy();
  t.after(stop);

  const r1 = await post(port, { messages: TURN1 });
  assert.equal(r1.status, 200);
  assert.equal(JSON.parse(await r1.text()).choices[0].message.content, 'Hello world');

  // The captured session id is persisted next to endpoint.json.
  const saved = JSON.parse(
    fs.readFileSync(path.join(stateHome, 'codex-proxy', 'sessions.json'), 'utf-8')
  );
  const entries = Object.values(saved);
  assert.equal(entries.length, 1);
  assert.equal(entries[0].sessionId, 'stub-session-1');
  assert.equal(entries[0].fedCount, 1);

  const r2 = await post(port, { messages: TURN2 });
  assert.equal(r2.status, 200);
  assert.equal(JSON.parse(await r2.text()).choices[0].message.content, 'resumed answer');

  const invocations = readInvocations(home);
  assert.equal(invocations.length, 2);
  assert.equal(invocations[0].resume, false);
  assert.equal(invocations[1].resume, true, 'second turn must use exec resume');
  assert.match(invocations[1].prompt, /beta second question/);
  assert.doesNotMatch(invocations[1].prompt, /alpha first question/);
});

test('resume failure falls back to a full-history run and still answers (#480)', async (t) => {
  const { port, home, stop } = await startProxy();
  t.after(stop);

  const r1 = await post(port, { messages: TURN1 });
  assert.equal(r1.status, 200);
  await r1.text();

  const turn2Fail = [
    { role: 'user', content: 'alpha first question' },
    { role: 'assistant', content: 'Hello world' },
    { role: 'user', content: 'please RESUMEFAIL now' }
  ];
  const r2 = await post(port, { messages: turn2Fail });
  assert.equal(r2.status, 200, 'a failed resume must not fail the request');
  assert.equal(JSON.parse(await r2.text()).choices[0].message.content, 'Hello world');

  const invocations = readInvocations(home);
  assert.equal(invocations.length, 3, 'expected fresh, resume, then full fallback');
  assert.equal(invocations[1].resume, true);
  assert.equal(invocations[2].resume, false);
  assert.match(invocations[2].prompt, /alpha first question/, 'fallback must feed the full history');
  assert.match(invocations[2].prompt, /RESUMEFAIL/);
});

test('changed system prompt does not resume; full run instead (#480)', async (t) => {
  const { port, home, stop } = await startProxy();
  t.after(stop);

  const t1 = [
    { role: 'system', content: 'be terse' },
    { role: 'user', content: 'alpha first question' }
  ];
  const r1 = await post(port, { messages: t1 });
  assert.equal(r1.status, 200);
  await r1.text();

  const t2 = [
    { role: 'system', content: 'be verbose' },
    { role: 'user', content: 'alpha first question' },
    { role: 'assistant', content: 'Hello world' },
    { role: 'user', content: 'beta second question' }
  ];
  const r2 = await post(port, { messages: t2 });
  assert.equal(r2.status, 200);
  assert.equal(JSON.parse(await r2.text()).choices[0].message.content, 'Hello world');

  const invocations = readInvocations(home);
  assert.equal(invocations.length, 2);
  assert.equal(invocations[1].resume, false, 'system change must bypass resume');
});

test('session map survives a proxy restart via sessions.json (#480)', async (t) => {
  const first = await startProxy();
  const r1 = await post(first.port, { messages: TURN1 });
  assert.equal(r1.status, 200);
  await r1.text();
  first.stop();

  const second = await startProxy(first);
  t.after(second.stop);
  const r2 = await post(second.port, { messages: TURN2 });
  assert.equal(r2.status, 200);
  assert.equal(JSON.parse(await r2.text()).choices[0].message.content, 'resumed answer');

  const invocations = readInvocations(first.home);
  assert.equal(invocations.length, 2);
  assert.equal(invocations[1].resume, true, 'restarted proxy must still resume');
});

const AS_ENV = (fake) => ({
  CODEX_PROXY_TRANSPORT: 'app-server',
  CODEX_APP_SERVER_SOCKET: fake.socketPath
});

test('app-server transport: two turns on one daemon thread, tail only on turn 2 (#480)', async (t) => {
  const fake = await createFakeAppServer();
  t.after(fake.close);
  const { port, stateHome, stop } = await startProxy(null, AS_ENV(fake));
  t.after(stop);

  const r1 = await post(port, { messages: TURN1 });
  assert.equal(r1.status, 200);
  const b1 = JSON.parse(await r1.text());
  assert.equal(b1.choices[0].message.content, 'app-server answer');
  assert.equal(b1.usage.prompt_tokens, 42, 'usage must map from thread/tokenUsage/updated last.inputTokens');
  assert.equal(b1.usage.completion_tokens, 5);

  const saved = JSON.parse(
    fs.readFileSync(path.join(stateHome, 'codex-proxy', 'sessions.json'), 'utf-8')
  );
  assert.equal(Object.values(saved)[0].threadId, 'stub-thread-1');

  const r2 = await post(port, { messages: TURN2 });
  assert.equal(r2.status, 200);
  assert.equal(JSON.parse(await r2.text()).choices[0].message.content, 'app-server answer');

  const starts = fake.requests.filter(r => r.method === 'thread/start');
  const turns = fake.requests.filter(r => r.method === 'turn/start');
  assert.equal(starts.length, 1, 'one daemon thread per conversation');
  assert.equal(turns.length, 2);
  assert.match(turns[0].text, /alpha first question/);
  assert.match(turns[1].text, /beta second question/);
  assert.doesNotMatch(turns[1].text, /alpha first question/, 'keyed turn must send only the new tail');
});

test('app-server transport: deltas stream as SSE chunks (#480)', async (t) => {
  const fake = await createFakeAppServer();
  t.after(fake.close);
  const { port, stop } = await startProxy(null, AS_ENV(fake));
  t.after(stop);
  const res = await post(port, { messages: TURN1, stream: true });
  assert.equal(res.status, 200);
  const sse = await res.text();
  assert.match(sse, /app-/);
  assert.match(sse, /server answer/);
  assert.match(sse, /"prompt_tokens":42/);
  assert.ok(sse.trimEnd().endsWith('data: [DONE]'));
});

test('app-server thread is resumed after a proxy restart via sessions.json (#480)', async (t) => {
  const fake = await createFakeAppServer();
  t.after(fake.close);
  const first = await startProxy(null, AS_ENV(fake));
  const r1 = await post(first.port, { messages: TURN1 });
  assert.equal(r1.status, 200);
  await r1.text();
  first.stop();

  const second = await startProxy(first, AS_ENV(fake));
  t.after(second.stop);
  const r2 = await post(second.port, { messages: TURN2 });
  assert.equal(r2.status, 200);
  assert.equal(JSON.parse(await r2.text()).choices[0].message.content, 'app-server answer');
  const resumes = fake.requests.filter(r => r.method === 'thread/resume');
  assert.equal(resumes.length, 1, 'restarted proxy must thread/resume the persisted thread');
  assert.equal(resumes[0].params.threadId, 'stub-thread-1');
  assert.equal(fake.requests.filter(r => r.method === 'thread/start').length, 1);
});

test('app-server turn failure falls back to exec and still answers (#480)', async (t) => {
  const fake = await createFakeAppServer();
  t.after(fake.close);
  const { port, stop } = await startProxy(null, AS_ENV(fake));
  t.after(stop);
  const res = await post(port, { messages: [{ role: 'user', content: 'please ASFAIL now' }] });
  assert.equal(res.status, 200, 'a failed app-server turn must not fail the request');
  assert.equal(JSON.parse(await res.text()).choices[0].message.content, 'Hello world');
});

test('app-server RPC failure falls back to exec and still answers (#480)', async (t) => {
  const fake = await createFakeAppServer();
  t.after(fake.close);
  const { port, stop } = await startProxy(null, AS_ENV(fake));
  t.after(stop);
  const res = await post(port, { messages: [{ role: 'user', content: 'please ASRPCFAIL now' }] });
  assert.equal(res.status, 200);
  assert.equal(JSON.parse(await res.text()).choices[0].message.content, 'Hello world');
});

test('daemon death mid-turn falls back to exec and still answers (#480)', async (t) => {
  const fake = await createFakeAppServer();
  t.after(fake.close);
  const { port, stop } = await startProxy(null, AS_ENV(fake));
  t.after(stop);
  const res = await post(port, { messages: [{ role: 'user', content: 'please ASKILL now' }] });
  assert.equal(res.status, 200, 'daemon death mid-turn must not fail the request');
  assert.equal(JSON.parse(await res.text()).choices[0].message.content, 'Hello world');
});

test('unreachable app-server socket falls back to exec and still answers (#480)', async (t) => {
  const { port, stop } = await startProxy(null, {
    CODEX_PROXY_TRANSPORT: 'app-server',
    CODEX_APP_SERVER_SOCKET: '/nonexistent/codex-proxy-test/app-server.sock'
  });
  t.after(stop);
  const res = await post(port, { messages: TURN1 });
  assert.equal(res.status, 200);
  assert.equal(JSON.parse(await res.text()).choices[0].message.content, 'Hello world');
});

test('CODEX_PROXY_TRANSPORT=exec-resume bypasses the app-server entirely (#480)', async (t) => {
  const fake = await createFakeAppServer();
  t.after(fake.close);
  const { port, stop } = await startProxy(null, {
    CODEX_PROXY_TRANSPORT: 'exec-resume',
    CODEX_APP_SERVER_SOCKET: fake.socketPath
  });
  t.after(stop);
  const res = await post(port, { messages: TURN1 });
  assert.equal(res.status, 200);
  assert.equal(JSON.parse(await res.text()).choices[0].message.content, 'Hello world');
  assert.equal(fake.requests.length, 0, 'exec-resume transport must not touch the daemon');
});
