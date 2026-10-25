#!/usr/bin/env node
// claude-code-proxy — OpenAI-compatible bridge to the Claude Code CLI.
//
// Same shape as the fleet's other warm bridges (bin/muse-proxy.mjs's serve
// pool, bin/commandcode-proxy.mjs's resident ACP process): one PERSISTENT
// `claude` process per conversation, never a spawn per request. Each
// conversation runs the CLI in bidirectional stream-json mode
// (`claude -p --input-format stream-json --output-format stream-json`), so
// a conversation's first turn pays the process start + full context once
// and later turns send only the newest user message — the CLI session
// itself holds the history, and Anthropic's prompt cache serves the
// replayed prefix.
//
// Conversations are keyed like commandcode-proxy keys them (explicit
// session headers / body ids, else a hash of the first user message), and
// the key -> CLI session map persists to sessions.json in the proxy dir,
// so a proxy restart re-attaches with `--resume <sessionId>` instead of
// re-bootstrapping. A request whose history no longer matches the mapped
// session (diverged prefix, extra mid-history user turns, lost session)
// is re-bootstrapped as a fresh session whose first turn is the full
// flattened transcript — the same text the cold path would send. A turn
// that fails before any content is committed is retried once that way.
//
// Auth is the CLI's own claude.ai OAuth login (~/.claude, under $HOME).
// ANTHROPIC_API_KEY is explicitly deleted from the child env: when set it
// outranks the account login and bills the pay-as-you-go API instead of
// the subscription. Children also get the sandbox's unauthenticated
// egress proxy + CA bundle pinned (fleet machines with direct egress set
// CLAUDE_CODE_PROXY_EGRESS="" in their start wrapper to disable that).
import http from 'node:http';
import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseProxyPort } from './proxy-port.mjs';

// Default port 18749: the next free port in the fleet family (clef 18742,
// agy 18743, codex 18744, antigravity-ls 18745, design-code 18746,
// muse 18747, commandcode 18748).
const PORT = parseProxyPort('CLAUDE_CODE_PROXY_PORT', 18749);
const HOST = process.env.CLAUDE_CODE_PROXY_HOST || '127.0.0.1';

const EGRESS_PROXY = process.env.CLAUDE_CODE_PROXY_EGRESS ?? 'http://hatch-egress-proxy:3128';
const EGRESS_CA = '/run/hatch/egress-tls/ca-bundle.pem';

const PROXY_DIR = path.dirname(fileURLToPath(import.meta.url));
const WORKROOT = process.env.CLAUDE_CODE_PROXY_WORKDIR || path.join(PROXY_DIR, 'work');
fs.mkdirSync(WORKROOT, { recursive: true });

const SESSION_FILE = process.env.CLAUDE_CODE_PROXY_SESSION_FILE || path.join(PROXY_DIR, 'sessions.json');
const MAX_LIVE_SESSIONS = 32;
const IDLE_TTL_MS = 2 * 60 * 60 * 1000;
const TURN_TIMEOUT_MS = 300 * 1000;

// The models the sandbox opencode.jsonc advertises for this provider.
// Any other requested id is passed through to the CLI untouched.
const MODELS = [{ id: 'claude-haiku-5-5', name: 'Haiku 5.5 (Claude Code)' }];
const DEFAULT_MODEL = 'claude-haiku-5-5';

const findClaudeBinary = () => {
  const candidates = [
    process.env.CLAUDE_CODE_BIN,
    `${process.env.HOME}/tools/claude-code/package/claude`,
    `${process.env.HOME}/.local/bin/claude`,
    'claude'
  ];
  for (const c of candidates) {
    if (c && (c === 'claude' || fs.existsSync(c))) return c;
  }
  return 'claude';
};
const CLAUDE_BIN = findClaudeBinary();

const resolveModel = (requested) => {
  let raw = (requested || DEFAULT_MODEL).trim();
  raw = raw.replace(/^.*\//, '').trim(); // strip any provider prefix
  return raw || DEFAULT_MODEL;
};

// Child env: inherit, but strip ANTHROPIC_API_KEY (it outranks the OAuth
// login) and pin the egress proxy + CA bundle the CLI needs here.
const childEnv = () => {
  const env = { ...process.env };
  delete env.ANTHROPIC_API_KEY;
  if (EGRESS_PROXY) {
    env.HTTPS_PROXY = EGRESS_PROXY;
    env.HTTP_PROXY = EGRESS_PROXY;
    env.https_proxy = EGRESS_PROXY;
    env.http_proxy = EGRESS_PROXY;
    env.NODE_USE_ENV_PROXY = '1';
  }
  if (EGRESS_PROXY && fs.existsSync(EGRESS_CA)) env.SSL_CERT_FILE = EGRESS_CA;
  env.NO_PROXY = [env.NO_PROXY, '127.0.0.1', 'localhost', '::1'].filter(Boolean).join(',');
  env.no_proxy = env.NO_PROXY;
  return env;
};

// ---------------------------------------------------------------------------
// Message helpers (mirror muse-proxy / commandcode-proxy)
// ---------------------------------------------------------------------------
const messageText = (m) => {
  if (!m || typeof m !== 'object') return '';
  if (typeof m.content === 'string') return m.content;
  if (Array.isArray(m.content)) {
    return m.content.map(c => (c && typeof c.text === 'string') ? c.text : '').join('\n');
  }
  return m.content ? JSON.stringify(m.content) : '';
};

const messageHash = (m) =>
  crypto.createHash('sha256').update(`${m.role}\x00${messageText(m)}`).digest('hex');

const formatTranscript = (messages) => {
  const convo = messages.filter(m => m.role !== 'system');
  return convo.map(m => {
    let role = m.role === 'assistant' ? 'Assistant' : (m.role === 'user' ? 'User' : m.role);
    let text = messageText(m);
    if (m.tool_calls && Array.isArray(m.tool_calls)) {
      const tcStr = m.tool_calls.map(tc => `[Tool Call: ${tc.function?.name || 'function'}(${tc.function?.arguments || ''})]`).join('\n');
      text = text ? `${text}\n${tcStr}` : tcStr;
    }
    if (m.role === 'tool') role = `Tool Result (${m.tool_call_id || 'result'})`;
    return `${role}: ${text}`;
  }).join('\n\n');
};

const systemText = (messages) =>
  messages.filter(m => m.role === 'system').map(messageText).join('\n\n');

// Conversation key derivation mirrors commandcode-proxy's getSessionKey:
// explicit ids first, then `user:`, then a hash of the first USER message
// (messages[0] is OpenCode's identical system prompt for every session
// and would collide all conversations).
const getSessionKey = (req, body) => {
  const explicitId =
    req.headers['x-conversation-id'] ||
    req.headers['x-session-id'] ||
    req.headers['conversation-id'] ||
    req.headers['session-id'] ||
    body.conversation_id ||
    body.conversationId ||
    body.session_id ||
    body.sessionId ||
    body.chat_id;
  if (explicitId && typeof explicitId === 'string' && explicitId.trim()) {
    return explicitId.trim();
  }
  if (body.user && typeof body.user === 'string' && body.user.trim()) {
    return `user:${body.user.trim()}`;
  }
  if (Array.isArray(body.messages) && body.messages.length > 0) {
    const firstUser = body.messages.find(m => m.role === 'user');
    const firstContent = firstUser ? messageText(firstUser) : '';
    if (firstContent.length > 0) {
      const hash = crypto.createHash('sha256').update(firstContent).digest('hex').slice(0, 16);
      return `turn0:${hash}`;
    }
  }
  return null;
};

// One stable workdir per conversation key: the CLI keys its on-disk
// project state by cwd slug, so a stable cwd keeps a conversation's
// transcript in a single ~/.claude/projects/<slug>/ directory.
const sessionWorkdirFor = (key) => {
  const hash = crypto.createHash('sha256').update(key).digest('hex').slice(0, 16);
  const dir = path.join(WORKROOT, 'sessions', hash);
  fs.mkdirSync(dir, { recursive: true });
  return dir;
};

// ---------------------------------------------------------------------------
// Persisted key -> CLI session map (survives proxy restarts; re-attached
// with --resume)
// ---------------------------------------------------------------------------
const sessionStore = new Map();
const saveSessions = () => {
  try {
    const obj = {};
    for (const [k, v] of sessionStore) obj[k] = v;
    fs.writeFileSync(`${SESSION_FILE}.tmp`, JSON.stringify(obj, null, 2) + '\n');
    fs.renameSync(`${SESSION_FILE}.tmp`, SESSION_FILE);
  } catch (err) {
    console.warn(`[claude-code-proxy] Failed to persist session store: ${err.message}`);
  }
};
const loadSessions = () => {
  try {
    const obj = JSON.parse(fs.readFileSync(SESSION_FILE, 'utf-8'));
    const now = Date.now();
    for (const [k, v] of Object.entries(obj)) {
      if (v && typeof v.sessionId === 'string' && now - (v.lastUsedAt || 0) <= IDLE_TTL_MS) {
        sessionStore.set(k, v);
      }
    }
    if (sessionStore.size > 0) {
      console.log(`[claude-code-proxy] Loaded ${sessionStore.size} warm-session mapping(s) from ${SESSION_FILE}`);
    }
  } catch { /* no state yet */ }
};
loadSessions();

// ---------------------------------------------------------------------------
// One resident `claude` stream-json process per conversation
// ---------------------------------------------------------------------------
class ClaudeSession {
  constructor({ key, model, sessionId, resume, system }) {
    this.key = key;
    this.model = model;
    this.sessionId = sessionId;
    this.child = null;
    this.ready = false;
    this.dead = null; // Error once the child is gone
    this.lineBuffer = '';
    this.stderrTail = '';
    this.turnWaiter = null; // { resolve, reject, onDelta, deltasText, lastAssistantText }
    this.workdir = sessionWorkdirFor(key);

    const args = [
      '-p',
      '--input-format', 'stream-json',
      '--output-format', 'stream-json',
      '--verbose',
      '--include-partial-messages',
      '--model', model
      // No --permission-mode: bypassPermissions is refused outright when
      // the CLI runs as root (the sandbox unit does), and default mode
      // routes tool approvals through can_use_tool control requests,
      // which _onMessage answers — see below.
    ];
    if (resume) args.push('--resume', sessionId);
    else args.push('--session-id', sessionId);
    if (!resume && system) args.push('--append-system-prompt', system);

    console.log(`[claude-code-proxy] Spawning session ${sessionId} (key ${key.slice(0, 24)}, ${resume ? 'resume' : 'fresh'}): ${CLAUDE_BIN} ${args.slice(0, 12).join(' ')}...`);
    const child = spawn(CLAUDE_BIN, args, {
      cwd: this.workdir,
      env: childEnv(),
      stdio: ['pipe', 'pipe', 'pipe']
    });
    this.child = child;

    this.initPromise = new Promise((resolve, reject) => {
      this._initResolve = resolve;
      this._initReject = reject;
      setTimeout(() => {
        if (!this.ready) {
          const err = new Error(`claude session ${sessionId} did not init within 30s`);
          this._initReject(err);
        }
      }, 30000).unref();
    });
    this.initPromise.catch(() => {}); // rejection is surfaced via runTurn

    // The stream-json protocol starts with an initialize control
    // request: the CLI emits nothing (not even system/init) until it
    // receives one (verified live against CLI 2.1.293).
    this.initRequestId = crypto.randomUUID();
    child.stdout.on('data', chunk => this._onStdout(chunk));
    child.stderr.on('data', chunk => {
      const msg = chunk.toString('utf-8');
      this.stderrTail = (this.stderrTail + msg).slice(-2000);
      if (msg.trim()) console.warn(`[claude-code-proxy stderr ${sessionId.slice(0, 8)}] ${msg.trim().slice(0, 300)}`);
    });
    child.on('error', err => this._onExit(`spawn error: ${err.message}`));
    child.on('close', (code, signal) => this._onExit(`claude exited (code ${code}, signal ${signal})`));
    try {
      this._write({
        type: 'control_request',
        request_id: this.initRequestId,
        request: { subtype: 'initialize' }
      });
    } catch (err) {
      this._onExit(`initialize write failed: ${err.message}`);
    }
  }

  _onExit(reason) {
    if (this.dead) return;
    this.dead = new Error(`${reason}${this.stderrTail ? `: ${this.stderrTail.trim().split('\n').pop()}` : ''}`);
    this.ready = false;
    if (this._initReject) this._initReject(this.dead);
    if (this.turnWaiter) {
      const w = this.turnWaiter;
      this.turnWaiter = null;
      w.reject(this.dead);
    }
  }

  _onStdout(chunk) {
    this.lineBuffer += chunk.toString('utf-8');
    const lines = this.lineBuffer.split('\n');
    this.lineBuffer = lines.pop();
    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed || !trimmed.startsWith('{')) continue;
      let msg;
      try { msg = JSON.parse(trimmed); } catch { continue; }
      this._onMessage(msg);
    }
  }

  _onMessage(msg) {
    if (msg.type === 'system' && msg.subtype === 'init') {
      if (typeof msg.session_id === 'string') this.sessionId = msg.session_id;
      this.ready = true;
      if (this._initResolve) this._initResolve(this.sessionId);
      return;
    }
    if (msg.type === 'control_response' &&
        msg.response?.request_id === this.initRequestId &&
        msg.response?.subtype === 'success') {
      // system/init follows immediately; treat the handshake answer as
      // readiness too so a turn is never gated on event ordering.
      this.ready = true;
      if (this._initResolve) this._initResolve(this.sessionId);
      return;
    }
    // Tool approvals arrive as can_use_tool control requests and a
    // request left unanswered would stall the turn. Answer allow, the
    // same posture as muse-proxy's serve sessions (approvalMode
    // allowAll): the CLI works in a per-conversation workdir under the
    // proxy's own workroot.
    if (msg.type === 'control_request' && msg.request?.subtype === 'can_use_tool') {
      this._write({
        type: 'control_response',
        response: {
          subtype: 'success',
          request_id: msg.request_id,
          response: { behavior: 'allow', updatedInput: msg.request.input || {} }
        }
      });
      return;
    }
    const w = this.turnWaiter;
    if (!w) return;
    if (msg.type === 'stream_event' && msg.event) {
      const ev = msg.event;
      if (ev.type === 'content_block_delta' && ev.delta?.type === 'text_delta' && typeof ev.delta.text === 'string') {
        w.deltasText += ev.delta.text;
        if (w.onDelta) { try { w.onDelta(ev.delta.text); } catch { /* response gone */ } }
      }
      return;
    }
    if (msg.type === 'assistant' && msg.message && Array.isArray(msg.message.content)) {
      const text = msg.message.content
        .filter(c => c && c.type === 'text' && typeof c.text === 'string')
        .map(c => c.text).join('');
      if (text) w.lastAssistantText = text;
      return;
    }
    if (msg.type === 'result') {
      this.turnWaiter = null;
      if (msg.subtype && msg.subtype !== 'success') {
        const detail = (typeof msg.result === 'string' && msg.result) ||
          (Array.isArray(msg.errors) && msg.errors.join('; ')) ||
          `result subtype ${msg.subtype}`;
        w.reject(new Error(`claude turn failed: ${String(detail).slice(0, 300)}`));
        return;
      }
      const usage = msg.usage || {};
      w.resolve({
        deltasText: w.deltasText,
        fullText: (typeof msg.result === 'string' && msg.result) || w.lastAssistantText || w.deltasText,
        promptTokens: typeof usage.input_tokens === 'number'
          ? usage.input_tokens + (usage.cache_read_input_tokens || 0) + (usage.cache_creation_input_tokens || 0)
          : undefined,
        completionTokens: typeof usage.output_tokens === 'number' ? usage.output_tokens : undefined
      });
    }
  }

  _write(obj) {
    if (!this.child || !this.child.stdin.writable) throw new Error('claude session stdin is not writable');
    this.child.stdin.write(JSON.stringify(obj) + '\n');
  }

  async runTurn(text, onDelta) {
    if (this.dead) throw this.dead;
    await this.initPromise;
    if (this.dead) throw this.dead;
    if (this.turnWaiter) throw new Error('claude session already has a turn in flight');
    return new Promise((resolve, reject) => {
      const waiter = {
        deltasText: '',
        lastAssistantText: null,
        onDelta,
        resolve: (v) => { clearTimeout(timer); resolve(v); },
        reject: (e) => { clearTimeout(timer); reject(e); }
      };
      const timer = setTimeout(() => {
        if (this.turnWaiter === waiter) {
          this.turnWaiter = null;
          this.kill(); // a wedged session must not poison later turns
          reject(new Error(`claude turn timed out after ${TURN_TIMEOUT_MS}ms`));
        }
      }, TURN_TIMEOUT_MS);
      this.turnWaiter = waiter;
      try {
        this._write({
          type: 'user',
          message: { role: 'user', content: [{ type: 'text', text }] },
          session_id: this.sessionId
        });
      } catch (err) {
        this.turnWaiter = null;
        waiter.reject(err);
      }
    });
  }

  kill() {
    if (this.child && !this.child.killed) {
      try { this.child.kill('SIGTERM'); } catch { /* already gone */ }
      const child = this.child;
      setTimeout(() => { try { child.kill('SIGKILL'); } catch { /* gone */ } }, 3000).unref();
    }
  }
}

// Live sessions + per-key serialization (turns on one CLI session must
// not interleave, and the delta decision needs the latest committed
// state).
const liveSessions = new Map();
const chains = new Map();
const enqueue = (key, job) => {
  const prev = chains.get(key) || Promise.resolve();
  const run = prev.then(job, job);
  chains.set(key, run.catch(() => {}));
  return run;
};

const evictIfNeeded = () => {
  const live = [...liveSessions.entries()]
    .filter(([, s]) => !s.dead)
    .sort((a, b) => (a[1].lastUsedAt || 0) - (b[1].lastUsedAt || 0));
  while (live.length >= MAX_LIVE_SESSIONS) {
    const [key, sess] = live.shift();
    console.log(`[claude-code-proxy] Evicting idle session for key ${key.slice(0, 24)}`);
    sess.kill();
    liveSessions.delete(key);
  }
  // Idle TTL sweep
  const now = Date.now();
  for (const [key, sess] of liveSessions) {
    if (now - (sess.lastUsedAt || 0) > IDLE_TTL_MS) {
      sess.kill();
      liveSessions.delete(key);
    }
  }
};
setInterval(evictIfNeeded, 60000).unref();

// Attempt the chat completion on the resident-session path. Mirrors
// muse-proxy's tryWarmChat contract: true when the response has been
// fully handled, false when nothing was written and the caller should
// retry once with a fresh full-transcript session.
const trySessionChat = async ({ key, messages, model, stream, res, chatId, created, forceFresh }) => {
  const hashes = messages.map(messageHash);
  let entry = forceFresh ? null : sessionStore.get(key);
  let sess = forceFresh ? null : liveSessions.get(key);
  if (sess && (sess.dead || sess.model !== model)) {
    sess.kill();
    liveSessions.delete(key);
    sess = null;
  }

  // Delta decision (against the stored mapping, whether or not a live
  // process exists): the stored prefix must match exactly and the only
  // new content allowed is one trailing user message.
  let turnText = null;
  if (entry && entry.model === model) {
    const last = messages[messages.length - 1];
    const suffix = messages.slice(entry.sentCount);
    const prefixOk = entry.sentCount > 0 &&
      entry.hashes.length >= entry.sentCount &&
      entry.hashes.slice(0, entry.sentCount).every((h, i) => h === hashes[i]);
    const midUsers = suffix.slice(0, -1).filter(m => m.role === 'user').length;
    if (prefixOk && suffix.length > 0 && last.role === 'user' && midUsers === 0) {
      turnText = messageText(last);
    }
  }

  if (sess && turnText === null) {
    console.log(`[claude-code-proxy] history diverged for key ${key.slice(0, 24)}; re-bootstrapping`);
    sess.kill();
    liveSessions.delete(key);
    sess = null;
    entry = null;
  }

  if (!sess) {
    evictIfNeeded();
    // A stored session may be re-attached with --resume only when the
    // delta decision passed; anything else starts fresh.
    const canResume = Boolean(entry && entry.sessionId && turnText !== null);
    const sessionId = canResume ? entry.sessionId : crypto.randomUUID();
    sess = new ClaudeSession({
      key,
      model,
      sessionId,
      resume: canResume,
      system: systemText(messages)
    });
    liveSessions.set(key, sess);
    if (!canResume) {
      // Fresh session: a lone user turn is sent as-is; anything longer
      // (re-bootstrap) goes as the full flattened transcript. The system
      // prompt rides --append-system-prompt at spawn instead.
      const nonSystem = messages.filter(m => m.role !== 'system');
      turnText = (nonSystem.length === 1 && nonSystem[0].role === 'user')
        ? messageText(nonSystem[0])
        : formatTranscript(messages);
    }
  }
  sess.lastUsedAt = Date.now();

  let headed = false;
  const writeChunk = (content) => {
    if (!stream || res.writableEnded) return;
    if (!headed) {
      headed = true;
      res.writeHead(200, {
        'Content-Type': 'text/event-stream; charset=utf-8',
        'Cache-Control': 'no-cache, no-transform',
        'Connection': 'keep-alive',
        'X-Accel-Buffering': 'no'
      });
    }
    res.write(`data: ${JSON.stringify({
      id: chatId, object: 'chat.completion.chunk', created, model,
      choices: [{ index: 0, delta: { content }, finish_reason: null }]
    })}\n\n`);
  };

  let result;
  try {
    result = await sess.runTurn(turnText, stream ? writeChunk : null);
  } catch (err) {
    // Dead/lost session: drop the mapping so the retry starts clean.
    liveSessions.delete(key);
    sessionStore.delete(key);
    saveSessions();
    if (!headed) return false;
    if (!res.writableEnded) {
      res.write(`data: ${JSON.stringify({ error: { message: `Claude Code execution failed: ${err.message}` } })}\n\n`);
      res.write('data: [DONE]\n\n');
    }
    res.end();
    return true;
  }

  // Commit the mapping only after a successful turn.
  const committed = {
    sessionId: sess.sessionId, model,
    hashes, sentCount: messages.length, lastUsedAt: Date.now()
  };
  sessionStore.set(key, committed);
  saveSessions();

  let fullContent = result.deltasText;
  let unstreamedSuffix = '';
  if (typeof result.fullText === 'string' && result.fullText.length >= fullContent.length) {
    if (result.fullText.startsWith(fullContent)) unstreamedSuffix = result.fullText.slice(fullContent.length);
    fullContent = result.fullText;
  }
  const promptTokens = (typeof result.promptTokens === 'number' && result.promptTokens > 0)
    ? result.promptTokens
    : Math.max(1, Math.ceil(turnText.length / 4));
  let completionTokens = (typeof result.completionTokens === 'number' && result.completionTokens > 0)
    ? result.completionTokens
    : 0;
  if (completionTokens === 0 && fullContent.length > 0) {
    completionTokens = Math.max(1, Math.ceil(fullContent.length / 4));
  }

  if (stream) {
    if (unstreamedSuffix.length > 0) writeChunk(unstreamedSuffix);
    if (!headed) {
      res.writeHead(200, {
        'Content-Type': 'text/event-stream; charset=utf-8',
        'Cache-Control': 'no-cache, no-transform',
        'Connection': 'keep-alive',
        'X-Accel-Buffering': 'no'
      });
    }
    res.write(`data: ${JSON.stringify({
      id: chatId, object: 'chat.completion.chunk', created, model,
      choices: [{ index: 0, delta: {}, finish_reason: 'stop' }],
      usage: { prompt_tokens: promptTokens, completion_tokens: completionTokens, total_tokens: promptTokens + completionTokens }
    })}\n\n`);
    res.write('data: [DONE]\n\n');
    res.end();
  } else {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({
      id: chatId, object: 'chat.completion', created, model,
      choices: [{ index: 0, message: { role: 'assistant', content: fullContent }, finish_reason: 'stop' }],
      usage: { prompt_tokens: promptTokens, completion_tokens: completionTokens, total_tokens: promptTokens + completionTokens }
    }, null, 2));
  }
  return true;
};

const server = http.createServer((req, res) => {
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  res.setHeader('Access-Control-Allow-Headers', '*');
  if (req.method === 'OPTIONS') { res.writeHead(204); res.end(); return; }

  const url = new URL(req.url, `http://${req.headers.host || HOST}`);
  const pathname = url.pathname.replace(/\/+$/, '') || '/';

  if (req.method === 'GET' && (pathname === '/health' || pathname === '/')) {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({
      status: 'ok',
      service: 'claude-code-proxy',
      port: PORT,
      binary: CLAUDE_BIN,
      engine: 'claude-stream-json-resident',
      defaultModel: DEFAULT_MODEL,
      endpoint: '/v1/chat/completions',
      sessions: {
        live: [...liveSessions.values()].filter(s => !s.dead).length,
        stored: sessionStore.size
      }
    }, null, 2));
    return;
  }

  if (req.method === 'GET' && (pathname === '/v1/models' || pathname === '/models')) {
    const now = Math.floor(Date.now() / 1000);
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({
      object: 'list',
      data: MODELS.map(m => ({ id: m.id, object: 'model', created: now, owned_by: 'anthropic-claude-code' }))
    }, null, 2));
    return;
  }

  if (req.method === 'POST' && (pathname === '/v1/chat/completions' || pathname === '/chat/completions')) {
    let bodyText = '';
    req.on('data', chunk => {
      bodyText += chunk;
      if (bodyText.length > 50 * 1024 * 1024) {
        res.writeHead(413, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: 'Request payload too large' } }));
        req.destroy();
      }
    });
    req.on('end', async () => {
      let body;
      try { body = JSON.parse(bodyText); } catch (err) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: `Invalid JSON body: ${err.message}` } }));
        return;
      }
      const messages = Array.isArray(body.messages) ? body.messages : [];
      if (messages.length === 0) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: 'Messages cannot be empty' } }));
        return;
      }
      const model = resolveModel(body.model);
      const stream = Boolean(body.stream);
      const chatId = `chatcmpl-claude-code-${crypto.randomUUID()}`;
      const created = Math.floor(Date.now() / 1000);
      const key = getSessionKey(req, body) || `anon:${crypto.randomUUID()}`;

      const job = async () => {
        let handled = await trySessionChat({ key, messages, model, stream, res, chatId, created, forceFresh: false });
        if (!handled && !res.writableEnded) {
          console.warn(`[claude-code-proxy] session path failed for key ${key.slice(0, 24)}; retrying once with a fresh session`);
          handled = await trySessionChat({ key, messages, model, stream, res, chatId, created, forceFresh: true });
        }
        if (!handled && !res.writableEnded) {
          res.writeHead(500, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ error: { message: 'Claude Code execution failed' } }));
        }
      };
      try {
        await enqueue(key, job);
      } catch (err) {
        if (!res.writableEnded && !res.headersSent) {
          res.writeHead(500, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ error: { message: `Claude Code execution failed: ${err.message}` } }));
        }
      }
    });
    return;
  }

  res.writeHead(404, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ error: { message: `Route not found: ${req.method} ${pathname}` } }));
});

// Publish the bound endpoint ($XDG_STATE_HOME/claude-code-proxy/
// endpoint.json) like the other fleet proxies, removed on exit.
const publishEndpoint = () => {
  try {
    const stateHome = process.env.XDG_STATE_HOME || path.join(os.homedir(), '.local', 'state');
    const dir = path.join(stateHome, 'claude-code-proxy');
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, 'endpoint.json');
    fs.writeFileSync(`${file}.tmp`, JSON.stringify({ host: HOST, port: PORT, pid: process.pid, url: `http://${HOST}:${PORT}` }) + '\n');
    fs.renameSync(`${file}.tmp`, file);
    const cleanup = () => { try { fs.unlinkSync(file); } catch {} };
    process.on('exit', cleanup);
    process.on('SIGTERM', () => { cleanup(); process.exit(0); });
    process.on('SIGINT', () => { cleanup(); process.exit(0); });
  } catch {}
};

server.listen(PORT, HOST, () => {
  publishEndpoint();
  console.log(`[claude-code-proxy] OpenAI-compatible bridge listening on http://${HOST}:${PORT}`);
  console.log(`[claude-code-proxy] Resolved claude binary: ${CLAUDE_BIN}`);
  console.log(`[claude-code-proxy] Workroot: ${WORKROOT}`);
});
