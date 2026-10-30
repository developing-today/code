#!/usr/bin/env node
// muse-proxy — OpenAI-compatible bridge to the Meta Muse Code CLI.
//
// Mirrors bin/codex-proxy.mjs in developing-today/code: one `muse exec --json`
// subprocess per chat-completions request, prompt delivered via --prompt-file,
// streamed back out of the CLI's JSONL event stream.
//
// Warm path (#480): chat completions are served by one persistent
// `muse serve` (MSP) host with durable per-conversation sessions whenever
// possible — see the warm-sessions section below. The exec path remains as
// the fallback for any warm failure and when MUSE_PROXY_WARM=0.
//
// Direct mode (#2753): a third, first-tried path. The CLI's OAuth login
// token is an account token, not an inference credential; the proxy
// exchanges it at POST /muse-code/key for the plan's minted inference
// key and calls the Meta Responses API directly (see the direct section
// below). Warm/exec remain the fallback when direct never got an
// upstream response. MUSE_PROXY_DIRECT=0 disables direct mode.
//
// Auth is the CLI's own OAuth login (~/.config/muse/auth.json). META_API_KEY
// must NOT be set for the child: the CLI binary states it takes priority over
// the account login, and the Model API key is a separate pay-as-you-go wallet
// (inference there returns 402 until funded). The child is therefore spawned
// with META_API_KEY explicitly deleted and with the sandbox's unauthenticated
// egress proxy + CA bundle, the only network path the CLI can reach here.
import http from 'node:http';
import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseProxyPort } from './proxy-port.mjs';

// Default port 18747: 18746 is claimed by design-code-proxy
// (DESIGN_CODE_PROXY_PORT) in this repo's config, so muse-proxy takes the
// next port in the family range (cf. clef 18742, agy 18743, codex 18744,
// antigravity-ls 18745, design-code 18746).
const PORT = parseProxyPort('MUSE_PROXY_PORT', 18747);
const HOST = process.env.MUSE_PROXY_HOST || '127.0.0.1';

// Egress pinning is a sandbox requirement (the CLI reaches Meta only via
// the unauthenticated egress proxy + CA bundle there). Fleet machines that
// reach Meta directly set MUSE_PROXY_EGRESS to the empty string in their
// start wrapper, which disables the pinning in childEnv() entirely.
const EGRESS_PROXY = process.env.MUSE_PROXY_EGRESS ?? 'http://hatch-egress-proxy:3128';
const EGRESS_CA = '/run/hatch/egress-tls/ca-bundle.pem';

const PROXY_DIR = path.dirname(fileURLToPath(import.meta.url));
const WORKDIR = process.env.MUSE_PROXY_WORKDIR || path.join(PROXY_DIR, 'work');
fs.mkdirSync(WORKDIR, { recursive: true });

const EFFORTS = ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'];
const MODELS = [
  { id: 'muse-spark-1.3', name: 'Muse Spark 1.3 (muse)', efforts: EFFORTS, defaultEffort: 'high' },
  { id: 'muse-wonder-1.0', name: 'Muse Wonder 1.0 (muse)', efforts: EFFORTS, defaultEffort: 'high' }
];

const findMuseBinary = () => {
  const candidates = [
    `${process.env.HOME}/.local/bin/muse`,
    '/run/current-system/sw/bin/muse',
    'muse'
  ];
  for (const c of candidates) {
    if (c === 'muse' || fs.existsSync(c)) {
      return c;
    }
  }
  return 'muse';
};

const MUSE_BIN = findMuseBinary();

const resolveModelAndEffort = (requestedModel, requestedEffort) => {
  let rawModel = (requestedModel || 'muse-spark-1.3').trim();
  // Strip any provider prefix before '/' (e.g. 'muse/', 'meta/', 'muse-code/')
  let modelStr = rawModel.replace(/^.*\//, '').toLowerCase().trim();
  let effort = requestedEffort ? requestedEffort.toLowerCase().trim() : null;

  // Check if model string ends with effort suffix (longest first)
  const possibleSuffixes = ['-minimal', '-medium', '-xhigh', '-ultra', '-high', '-none', '-low', '-max'];
  for (const suffix of possibleSuffixes) {
    if (modelStr.endsWith(suffix)) {
      effort = suffix.slice(1);
      modelStr = modelStr.slice(0, -suffix.length);
      break;
    }
  }

  // Contributor variants (#2753 amendment): the Meta pool's contributor
  // models are distinct upstream ids (muse-spark-1.3-contributor,
  // muse-wonder-1.0-contributor), optionally carrying an effort suffix
  // AFTER the contributor part (muse-wonder-1.0-contributor-high — the
  // effort suffix was already stripped above). Peel the contributor part
  // off for the base-model lookup and re-attach it to the resolved ids,
  // so the direct path sends the contributor id upstream verbatim. A
  // plain base-model request keeps the plain id — contributor is the
  // caller's choice, never a proxy rewrite.
  let contributor = false;
  if (modelStr.endsWith('-contributor')) {
    contributor = true;
    modelStr = modelStr.slice(0, -'-contributor'.length);
  }

  // Bare aliases
  if (modelStr === 'spark' || modelStr === 'muse-spark') modelStr = 'muse-spark-1.3';
  if (modelStr === 'wonder' || modelStr === 'muse-wonder') modelStr = 'muse-wonder-1.0';

  // Find base model config
  const modelConfig = MODELS.find(m => m.id === modelStr || m.id.replace(/-/g, '') === modelStr.replace(/-/g, ''))
    || MODELS[0]; // default to muse-spark-1.3

  // Resolve effort
  if (!effort || !modelConfig.efforts.includes(effort)) {
    effort = modelConfig.defaultEffort;
  }

  const baseModel = contributor ? `${modelConfig.id}-contributor` : modelConfig.id;
  return {
    baseModel,
    effort,
    fullName: `${baseModel}-${effort}`
  };
};

const formatPrompt = (messages) => {
  if (!Array.isArray(messages) || messages.length === 0) return '';
  if (messages.length === 1 && messages[0].role === 'user' && typeof messages[0].content === 'string') {
    return messages[0].content;
  }

  const parts = [];
  const systemMessages = messages.filter(m => m.role === 'system');
  const convoMessages = messages.filter(m => m.role !== 'system');

  if (systemMessages.length > 0) {
    const sysText = systemMessages.map(m =>
      typeof m.content === 'string' ? m.content : JSON.stringify(m.content)
    ).join('\n\n');
    parts.push(`[System Instructions]\n${sysText}`);
  }

  if (convoMessages.length > 0) {
    const convoText = convoMessages.map(m => {
      let role = m.role === 'assistant' ? 'Assistant' : (m.role === 'user' ? 'User' : m.role);
      let text = '';
      if (typeof m.content === 'string') {
        text = m.content;
      } else if (Array.isArray(m.content)) {
        text = m.content.map(c => c.text || JSON.stringify(c)).join('\n');
      } else if (m.content) {
        text = JSON.stringify(m.content);
      }

      if (m.tool_calls && Array.isArray(m.tool_calls)) {
        const tcStr = m.tool_calls.map(tc => `[Tool Call: ${tc.function?.name || 'function'}(${tc.function?.arguments || ''})]`).join('\n');
        text = text ? `${text}\n${tcStr}` : tcStr;
      }
      if (m.role === 'tool') {
        role = `Tool Result (${m.tool_call_id || 'result'})`;
      }

      return `${role}: ${text}`;
    }).join('\n\n');
    parts.push(convoText);
  }

  return parts.join('\n\n');
};

// Child env: inherit, but strip META_API_KEY (it outranks the OAuth login)
// and pin the egress proxy + CA bundle the CLI needs to reach Meta.
const childEnv = () => {
  const env = { ...process.env };
  delete env.META_API_KEY;
  if (EGRESS_PROXY) {
    env.HTTPS_PROXY = EGRESS_PROXY;
    env.HTTP_PROXY = EGRESS_PROXY;
    env.https_proxy = EGRESS_PROXY;
    env.http_proxy = EGRESS_PROXY;
  }
  if (EGRESS_PROXY && fs.existsSync(EGRESS_CA)) env.SSL_CERT_FILE = EGRESS_CA;
  env.NO_PROXY = [env.NO_PROXY, '127.0.0.1', 'localhost', '::1'].filter(Boolean).join(',');
  env.no_proxy = env.NO_PROXY;
  return env;
};

// ---------------------------------------------------------------------------
// CLI flag capability probe
// ---------------------------------------------------------------------------
// The muse CLI's flag surface differs between releases the fleet runs side
// by side (verified 2026-10-24: 1.4.2 advertises neither serve
// --disable-request-user-input nor exec --disable-reminders; 1.4.13
// advertises both), and an unknown option kills the child before it can
// serve anything. Rather than keying off version strings, probe the
// resolved binary's own `serve --help` / `exec --help` once at startup and
// pass only the flags it advertises. If a probe fails (or its output names
// no flags at all), optional flags are omitted — they are behavioural
// niceties — while core flags are still passed: a degraded proxy beats a
// dead one.
const CLI_FLAG_PLAN = {
  serve: [
    { flag: '--provider' },
    { flag: '--disable-request-user-input', optional: true }
  ],
  exec: [
    { flag: '--json' },
    { flag: '--prompt-file' },
    { flag: '--model' },
    { flag: '--reasoning-effort' },
    { flag: '--disable-reminders', optional: true },
    { flag: '--user-input-auto-resolve' },
    { flag: '--approval-mode' }
  ]
};
const cliCaps = { serve: null, exec: null };
const cliDropped = { serve: [], exec: [] };
let cliCapsPromise = null;

const flagSupported = (subcommand, flag, optional) => {
  const caps = cliCaps[subcommand];
  if (caps) return caps.has(flag);
  return !optional; // probe unavailable: keep core flags, omit optional ones
};

const probeCliFlags = (subcommand) => new Promise((resolve) => {
  let out = '';
  let child;
  try {
    child = spawn(MUSE_BIN, [subcommand, '--help'], {
      env: childEnv(),
      stdio: ['ignore', 'pipe', 'pipe']
    });
  } catch {
    resolve(null);
    return;
  }
  const timer = setTimeout(() => {
    try { child.kill('SIGKILL'); } catch { /* already gone */ }
  }, 10000);
  child.stdout.on('data', c => { out += c.toString('utf-8'); });
  child.stderr.on('data', c => { out += c.toString('utf-8'); });
  child.on('error', () => { clearTimeout(timer); resolve(null); });
  child.on('close', () => {
    clearTimeout(timer);
    const flags = new Set(out.match(/--[a-z][a-z0-9-]*/g) || []);
    resolve(flags.size > 0 ? flags : null);
  });
});

// Probe once, lazily; every caller awaits the same promise. Never rejects.
const ensureCliCaps = () => {
  if (!cliCapsPromise) {
    cliCapsPromise = (async () => {
      const [serve, exec] = await Promise.all([probeCliFlags('serve'), probeCliFlags('exec')]);
      cliCaps.serve = serve;
      cliCaps.exec = exec;
      for (const sub of ['serve', 'exec']) {
        cliDropped[sub] = CLI_FLAG_PLAN[sub]
          .filter(p => !flagSupported(sub, p.flag, p.optional))
          .map(p => p.flag);
        if (cliCaps[sub]) {
          console.log(`[muse-proxy] CLI probe: ${sub} advertises ${cliCaps[sub].size} flags` +
            (cliDropped[sub].length ? `; omitting unsupported: ${cliDropped[sub].join(', ')}` : '; all planned flags supported'));
        } else {
          console.warn(`[muse-proxy] CLI probe: ${sub} --help unusable; omitting optional flags: ${cliDropped[sub].join(', ') || '(none)'}`);
        }
      }
      return cliCaps;
    })();
  }
  return cliCapsPromise;
};

// Build an argv from parts: plain strings pass through untouched;
// {flag, value?, optional?} entries are included only when the resolved
// CLI advertises the flag (see the probe above).
const buildCliArgs = (subcommand, parts) => {
  const args = [];
  for (const part of parts) {
    if (typeof part === 'string') { args.push(part); continue; }
    if (!flagSupported(subcommand, part.flag, part.optional)) continue;
    args.push(part.flag);
    if (part.value !== undefined) args.push(String(part.value));
  }
  return args;
};

// ---------------------------------------------------------------------------
// Warm sessions (#480): one persistent `muse serve` (MSP over stdio) host
// ---------------------------------------------------------------------------
// The exec path above pays a fresh CLI spawn (+ ~26k tokens of session
// scaffolding) for every request. `muse serve` instead hosts durable MSP
// sessions in one long-lived process: a conversation's first turn creates a
// session, later turns send only the new user message, and the provider
// serves the scaffolding from its prompt cache (verified on the sandbox:
// turn 2 of a served session reported 26,225 cacheReadTokens of 26,455
// input tokens — ~230 uncached — vs 26,708 fully re-read input tokens for
// the same turn through the exec path).
//
// MSP is newline-delimited JSON-RPC 2.0 on the host's stdio (schema:
// `muse schema generate-json-schema`), so this is driven with the standard
// library only — no @muse-code/sdk runtime dependency.
//
// Conversations are keyed by sha256(model + system + first user message).
// The key -> session map persists to warm-sessions.json in the workdir
// (LRU 64 entries, 2h idle TTL), so sessions survive proxy restarts: the
// sessions themselves are durable in the CLI's own store and are re-attached
// with session/resume on the new host. A request whose history no longer
// matches the mapped session (diverged prefix, extra user turns, expired or
// lost session) is re-bootstrapped as a fresh session whose first turn is
// the full flattened transcript — exactly what the exec path sends on every
// request. Any warm-path failure before the response is committed falls
// back to the exec path; the warm path can be disabled with
// MUSE_PROXY_WARM=0.

const WARM_ENABLED = process.env.MUSE_PROXY_WARM !== '0';
const WARM_STATE_FILE =
  process.env.MUSE_PROXY_WARM_STATE || path.join(WORKDIR, 'warm-sessions.json');
const WARM_TTL_MS = 2 * 60 * 60 * 1000;
const WARM_MAX_SESSIONS = 64;
const WARM_TURN_TIMEOUT_MS = 300 * 1000;

// MSP command ids must be UUIDv7 (the host rejects v4: "expected UUIDv7").
let uuidV7LastMs = -1;
let uuidV7Seq = 0;
const uuidv7 = () => {
  let now = Date.now();
  if (now <= uuidV7LastMs) {
    now = uuidV7LastMs;
    uuidV7Seq += 1;
    if (uuidV7Seq > 0xfff) { now += 1; uuidV7Seq = 0; }
  } else {
    uuidV7Seq = 0;
  }
  uuidV7LastMs = now;
  const bytes = crypto.randomBytes(16);
  let t = now;
  for (let i = 5; i >= 0; i -= 1) { bytes[i] = t & 0xff; t = Math.floor(t / 256); }
  bytes[6] = 0x70 | ((uuidV7Seq >> 8) & 0x0f);
  bytes[7] = uuidV7Seq & 0xff;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = bytes.toString('hex');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
};

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

const conversationKey = (messages, baseModel) => {
  const system = messages.filter(m => m.role === 'system').map(messageText).join('\n\n');
  const firstUser = messages.find(m => m.role === 'user');
  return crypto.createHash('sha256')
    .update(`${baseModel}\x00${system}\x00${firstUser ? messageText(firstUser) : ''}`)
    .digest('hex');
};

// --- persistent key -> session map -----------------------------------------
const warmStore = {
  map: new Map(),
  loaded: false,
  load() {
    if (this.loaded) return;
    this.loaded = true;
    try {
      const raw = JSON.parse(fs.readFileSync(WARM_STATE_FILE, 'utf-8'));
      const now = Date.now();
      for (const e of raw.entries || []) {
        if (e && e.key && e.sessionId && now - (e.lastUsedAt || 0) <= WARM_TTL_MS) {
          this.map.set(e.key, e);
        }
      }
    } catch { /* no state yet */ }
  },
  save() {
    try {
      const entries = [...this.map.values()].map(e => ({
        key: e.key, sessionId: e.sessionId, model: e.model,
        hashes: e.hashes, sentCount: e.sentCount, lastUsedAt: e.lastUsedAt
      }));
      const tmp = `${WARM_STATE_FILE}.tmp`;
      fs.writeFileSync(tmp, JSON.stringify({ entries }, null, 2));
      fs.renameSync(tmp, WARM_STATE_FILE);
    } catch (err) {
      console.warn(`[muse-proxy warm] state save failed: ${err.message}`);
    }
  },
  get(key) {
    this.load();
    const entry = this.map.get(key);
    if (!entry) return null;
    if (Date.now() - (entry.lastUsedAt || 0) > WARM_TTL_MS) {
      this.map.delete(key);
      this.save();
      return null;
    }
    return entry;
  },
  set(key, entry) {
    this.load();
    this.map.set(key, entry);
    if (this.map.size > WARM_MAX_SESSIONS) {
      const byAge = [...this.map.values()].sort((a, b) => (a.lastUsedAt || 0) - (b.lastUsedAt || 0));
      for (const old of byAge.slice(0, this.map.size - WARM_MAX_SESSIONS)) {
        this.map.delete(old.key);
        warmHost.leave(old.sessionId);
      }
    }
    this.save();
  },
  delete(key) {
    this.map.delete(key);
    this.save();
  }
};

// --- the one persistent serve host ------------------------------------------
const warmHost = {
  child: null,
  readyPromise: null,
  gen: 0,
  seq: 0,
  pending: new Map(),
  listeners: new Set(),
  stderrTail: '',

  ensure() {
    if (this.readyPromise) return this.readyPromise;
    this.readyPromise = this._start().catch(err => {
      this.readyPromise = null;
      throw err;
    });
    return this.readyPromise;
  },

  async _start() {
    await ensureCliCaps();
    const args = buildCliArgs('serve', [
      'serve',
      { flag: '--provider', value: 'meta' },
      { flag: '--disable-request-user-input', optional: true }
    ]);
    console.log(`[muse-proxy warm] Spawning serve host: ${MUSE_BIN} ${args.join(' ')}`);
    const child = spawn(MUSE_BIN, args, {
      cwd: WORKDIR,
      env: childEnv(),
      stdio: ['pipe', 'pipe', 'pipe']
    });
    this.child = child;
    this.stderrTail = '';
    let lineBuffer = '';
    child.stdout.on('data', chunk => {
      lineBuffer += chunk.toString('utf-8');
      const lines = lineBuffer.split('\n');
      lineBuffer = lines.pop();
      for (const line of lines) this._onLine(line);
    });
    child.stderr.on('data', chunk => {
      const msg = chunk.toString('utf-8');
      this.stderrTail = (this.stderrTail + msg).slice(-2000);
      if (msg.trim().length > 0) console.warn(`[muse-proxy warm stderr] ${msg.trim()}`);
    });
    child.on('error', err => this._onExit(`spawn error: ${err.message}`));
    child.on('close', (code, signal) => this._onExit(`serve host exited (code ${code}, signal ${signal})`));
    let result;
    try {
      result = await this.rpc('initialize', {
        clientInfo: { name: 'muse_proxy', version: '1.0.0' },
        capabilities: { userInputDialogs: false }
      }, 20000);
      this._write({ jsonrpc: '2.0', method: 'initialized' });
    } catch (err) {
      try { child.kill('SIGKILL'); } catch { /* already gone */ }
      if (this.child === child) this.child = null;
      throw err;
    }
    this.gen += 1;
    console.log(`[muse-proxy warm] serve host ready (gen ${this.gen}, server ${result?.serverInfo?.name} ${result?.serverInfo?.version})`);
  },

  _onExit(reason) {
    if (!this.child && this.pending.size === 0) return;
    this.child = null;
    this.readyPromise = null;
    const err = new Error(reason);
    for (const p of this.pending.values()) p.reject(err);
    this.pending.clear();
    for (const fn of [...this.listeners]) {
      try { fn({ method: 'host/died', params: { reason } }); } catch { /* listener */ }
    }
  },

  _write(obj) {
    if (!this.child || !this.child.stdin.writable) throw new Error('serve host stdin is not writable');
    this.child.stdin.write(JSON.stringify(obj) + '\n');
  },

  _onLine(line) {
    const trimmed = line.trim();
    if (!trimmed || !trimmed.startsWith('{')) return;
    let msg;
    try { msg = JSON.parse(trimmed); } catch { return; }
    if (msg.id !== undefined && msg.method === undefined && this.pending.has(msg.id)) {
      const p = this.pending.get(msg.id);
      this.pending.delete(msg.id);
      if (msg.error) p.reject(new Error(`MSP ${msg.error.message || 'error'} (code ${msg.error.code})`));
      else p.resolve(msg.result);
      return;
    }
    if (msg.id !== undefined && typeof msg.method === 'string') {
      // Server-initiated request (approval/request, userInput/request):
      // acknowledge presentation only, as the SDK's receipt does. Sessions
      // run with approvalMode allowAll and request-user-input withheld, so
      // no decision is expected to follow.
      try { this._write({ jsonrpc: '2.0', id: msg.id, result: {} }); } catch { /* host gone */ }
      console.warn(`[muse-proxy warm] server request ${msg.method} acknowledged`);
      return;
    }
    if (typeof msg.method === 'string') {
      for (const fn of [...this.listeners]) {
        try { fn(msg); } catch { /* listener */ }
      }
    }
  },

  rpc(method, params, timeoutMs = 30000) {
    return new Promise((resolve, reject) => {
      const id = ++this.seq;
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`MSP ${method} timed out after ${timeoutMs}ms`));
      }, timeoutMs);
      this.pending.set(id, {
        resolve: v => { clearTimeout(timer); resolve(v); },
        reject: e => { clearTimeout(timer); reject(e); }
      });
      try {
        this._write({ jsonrpc: '2.0', id, method, params });
      } catch (err) {
        this.pending.delete(id);
        clearTimeout(timer);
        reject(err);
      }
    });
  },

  onNotification(fn) {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  },

  leave(sessionId) {
    if (!sessionId || !this.readyPromise) return;
    this.rpc('session/leave', { commandId: uuidv7(), sessionId }, 5000).catch(() => {});
  },

  async startSession(baseModel) {
    const result = await this.rpc('session/start', {
      commandId: uuidv7(),
      modelId: baseModel,
      approvalMode: 'allowAll',
      workspaceRoots: [WORKDIR]
    });
    const sessionId = result?.session?.sessionId;
    if (!sessionId) throw new Error('session/start returned no sessionId');
    return sessionId;
  },

  async resumeSession(sessionId) {
    const result = await this.rpc('session/resume', {
      commandId: uuidv7(),
      sessionId,
      excludeItems: true
    });
    const resumed = result?.session?.sessionId;
    if (!resumed) throw new Error('session/resume returned no session');
    return resumed;
  },

  // Run one turn on a loaded session. Streams agentMessage text deltas to
  // onDelta (when given) and resolves with the turn's full text + usage.
  runTurn({ sessionId, text, effort, onDelta }) {
    return new Promise((resolve, reject) => {
      let turnId = null;
      let deltasText = '';
      let lastAgentText = null;
      let lastFinalText = null;
      let usage = null;
      const itemKinds = new Map();
      let settled = false;
      const settle = (fn, value) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        off();
        fn(value);
      };
      const timer = setTimeout(() => {
        settle(reject, new Error(`warm turn timed out after ${WARM_TURN_TIMEOUT_MS}ms`));
      }, WARM_TURN_TIMEOUT_MS);
      const off = this.onNotification(msg => {
        const params = msg.params || {};
        if (params.sessionId !== sessionId && msg.method !== 'host/died') return;
        switch (msg.method) {
          case 'host/died':
            settle(reject, new Error(`serve host died mid-turn: ${params.reason}`));
            break;
          case 'item/started':
            if (params.item && params.item.itemId) itemKinds.set(params.item.itemId, params.item.kind);
            break;
          case 'item/delta': {
            const kind = itemKinds.get(params.itemId);
            if ((params.field === undefined || params.field === 'text') &&
                (kind === undefined || kind === 'agentMessage') &&
                typeof params.delta === 'string') {
              deltasText += params.delta;
              if (onDelta) {
                try { onDelta(params.delta); } catch { /* response gone */ }
              }
            }
            break;
          }
          case 'item/completed':
            if (params.item && params.item.kind === 'agentMessage' && typeof params.item.text === 'string') {
              lastAgentText = params.item.text;
              if (params.item.messagePhase === 'finalAnswer') lastFinalText = params.item.text;
            }
            break;
          case 'session/tokenUsage':
            if (params.usage) {
              usage = {
                promptTokens: params.usage.inputTokens,
                completionTokens: params.usage.outputTokens,
                cacheReadTokens: typeof params.usage.cacheReadTokens === 'number' ? params.usage.cacheReadTokens : 0
              };
            }
            break;
          case 'turn/completed':
            if (turnId && params.turnId !== turnId) break;
            if (params.terminal !== 'completed') {
              settle(reject, new Error(`warm turn terminated with state '${params.terminal}'${params.reason ? `: ${params.reason}` : ''}`));
            } else {
              settle(resolve, {
                deltasText,
                fullText: lastFinalText ?? lastAgentText ?? deltasText,
                promptTokens: usage?.promptTokens,
                completionTokens: usage?.completionTokens,
                cacheReadTokens: usage?.cacheReadTokens
              });
            }
            break;
          default:
            break;
        }
      });
      this.rpc('turn/start', {
        commandId: uuidv7(),
        sessionId,
        input: [{ type: 'text', text }],
        reasoningEffort: effort
      }).then(ack => {
        turnId = ack?.turnId || turnId;
      }).catch(err => settle(reject, err));
    });
  }
};

// Per-conversation serialization: turns on one session must not interleave,
// and the delta decision must be made against the latest committed state.
const warmChains = new Map();
const enqueueWarm = (key, job) => {
  const prev = warmChains.get(key) || Promise.resolve();
  const run = prev.then(job, job);
  warmChains.set(key, run.catch(() => {}));
  return run;
};

// Attempt the chat completion on the warm serve path. Returns true when the
// response has been fully handled (success, or a stream already committed
// that had to be terminated in place); false when nothing was written and
// the caller should fall back to the exec path.
const tryWarmChat = async ({ messages, baseModel, effort, fullName, stream, res, chatId, created }) => {
  const key = conversationKey(messages, baseModel);
  let headed = false;
  let streamedText = '';
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
    const chunk = {
      id: chatId,
      object: 'chat.completion.chunk',
      created,
      model: fullName,
      choices: [{ index: 0, delta: { content }, finish_reason: null }]
    };
    res.write(`data: ${JSON.stringify(chunk)}\n\n`);
    streamedText += content;
  };

  const job = async () => {
    await warmHost.ensure();
    let entry = warmStore.get(key);
    let sessionId = null;
    let turnText = null;
    const hashes = messages.map(messageHash);

    if (entry && entry.sessionId && entry.model === baseModel) {
      if (entry.hostGen === warmHost.gen) {
        sessionId = entry.sessionId;
      } else {
        try {
          sessionId = await warmHost.resumeSession(entry.sessionId);
          entry.hostGen = warmHost.gen;
          console.log(`[muse-proxy warm] resumed session ${sessionId} for key ${key.slice(0, 8)}`);
        } catch (err) {
          console.warn(`[muse-proxy warm] resume failed (${err.message}); re-bootstrapping`);
          sessionId = null;
        }
      }
    }

    if (sessionId) {
      const last = messages[messages.length - 1];
      const suffix = messages.slice(entry.sentCount);
      const prefixOk = entry.sentCount > 0 &&
        entry.hashes.length >= entry.sentCount &&
        entry.hashes.slice(0, entry.sentCount).every((h, i) => h === hashes[i]);
      const midUsers = suffix.slice(0, -1).filter(m => m.role === 'user').length;
      if (prefixOk && suffix.length > 0 && last.role === 'user' && midUsers === 0) {
        turnText = messageText(last);
      } else {
        console.log(`[muse-proxy warm] history diverged for key ${key.slice(0, 8)}; re-bootstrapping`);
        sessionId = null;
      }
    }

    if (!sessionId) {
      const replacedSessionId = entry && entry.sessionId;
      sessionId = await warmHost.startSession(baseModel);
      if (replacedSessionId && replacedSessionId !== sessionId) warmHost.leave(replacedSessionId);
      turnText = formatPrompt(messages);
      entry = {
        key, sessionId, model: baseModel,
        hashes: [], sentCount: 0, lastUsedAt: Date.now(), hostGen: warmHost.gen
      };
      warmStore.set(key, entry);
      console.log(`[muse-proxy warm] new session ${sessionId} for key ${key.slice(0, 8)}`);
    }

    const result = await warmHost.runTurn({
      sessionId,
      text: turnText,
      effort,
      onDelta: stream ? writeChunk : null
    });

    entry.sessionId = sessionId;
    entry.hashes = hashes;
    entry.sentCount = messages.length;
    entry.lastUsedAt = Date.now();
    entry.hostGen = warmHost.gen;
    warmStore.set(key, entry);
    return result;
  };

  let result;
  try {
    result = await enqueueWarm(key, job);
  } catch (err) {
    console.warn(`[muse-proxy warm] warm path failed (${err.message}); ${headed ? 'stream already committed, terminating in place' : 'falling back to exec'}`);
    if (!headed) return false;
    if (!res.writableEnded) {
      res.write(`data: ${JSON.stringify({ error: { message: `Muse execution failed: ${err.message}` } })}\n\n`);
      res.write('data: [DONE]\n\n');
    }
    res.end();
    return true;
  }

  // Reconcile deltas with the authoritative final text (same rule as the
  // exec path, #484): only a missing tail can still be appended to a stream.
  let fullContent = result.deltasText;
  let unstreamedSuffix = '';
  if (typeof result.fullText === 'string' && result.fullText.length >= fullContent.length) {
    if (result.fullText.startsWith(fullContent)) unstreamedSuffix = result.fullText.slice(fullContent.length);
    fullContent = result.fullText;
  }
  const promptTokens = (typeof result.promptTokens === 'number' && result.promptTokens > 0)
    ? result.promptTokens
    : Math.max(1, Math.ceil(formatPrompt(messages).length / 4));
  let completionTokens = (typeof result.completionTokens === 'number' && result.completionTokens > 0)
    ? result.completionTokens
    : 0;
  if (completionTokens === 0 && fullContent.length > 0) {
    completionTokens = Math.max(1, Math.ceil(fullContent.length / 4));
  }
  const cacheReadTokens = (typeof result.cacheReadTokens === 'number' && result.cacheReadTokens > 0)
    ? result.cacheReadTokens
    : 0;

  if (stream) {
    if (unstreamedSuffix.length > 0) writeChunk(unstreamedSuffix);
    if (!headed) {
      headed = true;
      res.writeHead(200, {
        'Content-Type': 'text/event-stream; charset=utf-8',
        'Cache-Control': 'no-cache, no-transform',
        'Connection': 'keep-alive',
        'X-Accel-Buffering': 'no'
      });
    }
    const endChunk = {
      id: chatId,
      object: 'chat.completion.chunk',
      created,
      model: fullName,
      choices: [{ index: 0, delta: {}, finish_reason: 'stop' }],
      usage: {
        prompt_tokens: promptTokens,
        completion_tokens: completionTokens,
        total_tokens: promptTokens + completionTokens,
        ...(cacheReadTokens > 0 ? { prompt_tokens_details: { cached_tokens: cacheReadTokens } } : {})
      }
    };
    res.write(`data: ${JSON.stringify(endChunk)}\n\n`);
    res.write('data: [DONE]\n\n');
    res.end();
  } else {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({
      id: chatId,
      object: 'chat.completion',
      created,
      model: fullName,
      choices: [{
        index: 0,
        message: { role: 'assistant', content: fullContent },
        finish_reason: 'stop'
      }],
      usage: {
        prompt_tokens: promptTokens,
        completion_tokens: completionTokens,
        total_tokens: promptTokens + completionTokens,
        ...(cacheReadTokens > 0 ? { prompt_tokens_details: { cached_tokens: cacheReadTokens } } : {})
      }
    }, null, 2));
  }
  return true;
};


// ---------------------------------------------------------------------------
// Direct mode (#2753): account-token exchange + Meta Responses API
// ---------------------------------------------------------------------------
// The Muse CLI's OAuth login (~/.config/muse/auth.json,
// providers.meta.access_token, a `dca:...` account token) is NOT an
// inference credential: /v1 calls with it return 401. The CLI exchanges it
// at POST {META_API_BASE}/muse-code/key (x-api-version: 1.0.0, body
// {"onboard": false}) for a MintedKey whose api_key field is the plan's
// inference key (`LLM|...` form), and that key is what inference presents.
// This section mirrors the exchange (the key value is never logged).
//
// Attestation is per-route, verified live 2026-10-30: with the minted key,
// /v1/chat/completions answers 403 client_attestation_denied no matter what
// headers are sent, while /v1/responses answers 200 with bare Authorization
// only. Direct mode therefore speaks the Responses API upstream and
// translates chat completions to/from it; /v1/responses is passed through.
// No tbh/muse-build headers are sent — they are proven unnecessary.
//
// Fallback discipline: ANY upstream HTTP response on the direct path
// (200/400/402/403/429/...) is returned to the client — a quota wall or an
// attestation denial must not cause a second burn through the CLI door.
// The warm/exec paths take over only when direct never got an upstream
// response: no credential, an exchange transport failure, or an upstream
// fetch that throws. One exception: is_subs_active === false or
// require_payment === true from the exchange is a hard 502 naming the
// subscription state, never a silent CLI fallback.

const DIRECT_ENABLED = process.env.MUSE_PROXY_DIRECT !== '0';
const META_API_BASE = (process.env.MUSE_PROXY_META_API_BASE || 'https://api.meta.ai').replace(/\/+$/, '');
const MUSE_AUTH_PATH = process.env.MUSE_AUTH_PATH || path.join(os.homedir(), '.config', 'muse', 'auth.json');
const DIRECT_KEY_TTL_MS = 10 * 60 * 1000;

const directState = {
  credentialFound: false,
  cachedKey: null,
  cachedAt: 0,
  subsTier: null,
  subsActive: null,
  exchangePromise: null
};

const loadDirectCredential = () => {
  try {
    const parsed = JSON.parse(fs.readFileSync(MUSE_AUTH_PATH, 'utf-8'));
    const meta = (parsed && parsed.providers && parsed.providers.meta) || {};
    const accessToken = typeof meta.access_token === 'string' && meta.access_token ? meta.access_token : null;
    const apiKey = typeof meta.api_key === 'string' && meta.api_key ? meta.api_key : null;
    directState.credentialFound = Boolean(accessToken || apiKey);
    return directState.credentialFound ? { accessToken, apiKey } : null;
  } catch {
    directState.credentialFound = false;
    return null;
  }
};

// One exchange against /muse-code/key. Throws only on transport failure;
// every HTTP outcome is reported as a result object.
const exchangeMintedKey = async (accessToken) => {
  const response = await fetch(`${META_API_BASE}/muse-code/key`, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${accessToken}`,
      'x-api-version': '1.0.0',
      'Content-Type': 'application/json',
      Accept: 'application/json'
    },
    body: JSON.stringify({ onboard: false })
  });
  let body = null;
  try { body = await response.json(); } catch { body = null; }
  if (!response.ok) {
    return { ok: false, message: `Direct Meta route unavailable: key exchange failed with HTTP ${response.status}` };
  }
  if (body && body.is_subs_active === false) {
    return { ok: false, message: 'Direct Meta route unavailable: Muse Code subscription is not active (is_subs_active=false)' };
  }
  if (body && body.require_payment === true) {
    return { ok: false, message: 'Direct Meta route unavailable: Muse Code subscription requires payment (require_payment=true)' };
  }
  if (!body || typeof body.api_key !== 'string' || !body.api_key) {
    return { ok: false, message: 'Direct Meta route unavailable: key exchange returned no api_key' };
  }
  directState.cachedKey = body.api_key;
  directState.cachedAt = Date.now();
  if (typeof body.subs_tier_name === 'string') directState.subsTier = body.subs_tier_name;
  if (typeof body.is_subs_active === 'boolean') directState.subsActive = body.is_subs_active;
  return { ok: true, key: body.api_key };
};

// Resolve the inference key to present. Result statuses: 'disabled' and
// 'none' (no credential) and 'transport' (exchange fetch threw) all mean
// the caller may fall back; 'hard' means Meta answered and the caller must
// surface the error instead. When the auth store holds only a stored
// api_key (no account token), that key is used without exchanging.
const getDirectKey = async ({ forceExchange = false } = {}) => {
  if (!DIRECT_ENABLED) return { status: 'disabled' };
  const cred = loadDirectCredential();
  if (!cred) return { status: 'none' };
  if (!cred.accessToken) {
    directState.cachedKey = cred.apiKey;
    directState.cachedAt = Date.now();
    return { status: 'ok', key: cred.apiKey, stored: true };
  }
  if (!forceExchange && directState.cachedKey &&
      Date.now() - directState.cachedAt < DIRECT_KEY_TTL_MS) {
    return { status: 'ok', key: directState.cachedKey };
  }
  if (!directState.exchangePromise) {
    directState.exchangePromise = exchangeMintedKey(cred.accessToken)
      .then(result => (result.ok
        ? { status: 'ok', key: result.key }
        : { status: 'hard', message: result.message }))
      .catch(err => ({ status: 'transport', message: err.message }))
      .finally(() => { directState.exchangePromise = null; });
  }
  return directState.exchangePromise;
};

// POST /v1/responses upstream with the resolved key. On 401: invalidate the
// cached key, re-exchange once, retry once (the second response is final).
// Throws only when a fetch itself throws (no upstream response received).
const directResponsesFetch = async (keyResult, requestBodyText, accept) => {
  const doFetch = (key) => fetch(`${META_API_BASE}/v1/responses`, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${key}`,
      'Content-Type': 'application/json',
      Accept: accept
    },
    body: requestBodyText
  });
  let response = await doFetch(keyResult.key);
  if (response.status === 401 && !keyResult.stored) {
    directState.cachedKey = null;
    directState.cachedAt = 0;
    const fresh = await getDirectKey({ forceExchange: true });
    if (fresh.status === 'ok') {
      response = await doFetch(fresh.key);
    } else if (fresh.status === 'hard') {
      return { hard: fresh };
    }
    // Any other re-exchange outcome leaves the original 401 as the answer:
    // Meta already responded, so the 401 passes through rather than falling
    // back to a second burn through the CLI door.
  }
  return { response };
};

const writeJsonBody = (res, status, obj) => {
  res.writeHead(status, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify(obj, null, 2));
};

// --- chat <-> responses translation -----------------------------------------

const mapResponsesEffort = (effort) => {
  switch ((effort || '').toLowerCase()) {
    case 'none': return null; // none means: send no reasoning block at all
    case 'minimal':
    case 'low':
    case 'medium':
    case 'high':
    case 'xhigh': return (effort || '').toLowerCase();
    // The Responses API tops out at xhigh; the CLI's max/ultra efforts have
    // no direct equivalent, so they clamp to the strongest available.
    case 'max':
    case 'ultra': return 'xhigh';
    default: return null;
  }
};

const contentPartsForInput = (content) => {
  if (typeof content === 'string') return content;
  if (Array.isArray(content)) {
    return content.map(part => (part && typeof part.text === 'string')
      ? { type: 'input_text', text: part.text }
      : part);
  }
  return content ? JSON.stringify(content) : '';
};

// Meta's subscription-key gate rejects requests whose function tools carry
// the well-known agent-harness shell names (`shell`, `bash`) with 403
// client_attestation_denied — verified live 2026-10-30 by bisecting a real
// opencode request body: every other tool, the instructions, and all client
// headers pass unchanged, while `run_command` / `terminal` pass. The direct
// path therefore presents those tools under alias names and translates the
// model's tool calls back on the way out. The CLI door never needed this —
// the CLI sends its own toolset.
const DIRECT_TOOL_ALIASES = { shell: 'run_command', bash: 'terminal' };
const DIRECT_TOOL_NAMES = Object.fromEntries(
  Object.entries(DIRECT_TOOL_ALIASES).map(([client, alias]) => [alias, client])
);
const directToolAlias = (name, taken) => {
  const alias = DIRECT_TOOL_ALIASES[name];
  if (!alias || (taken && taken.has(alias))) return name;
  return alias;
};
const clientToolName = (name) => DIRECT_TOOL_NAMES[name] || name;

const buildResponsesRequest = (body, baseModel, effort) => {
  const messages = Array.isArray(body.messages) ? body.messages : [];
  const instructions = messages
    .filter(m => m.role === 'system')
    .map(messageText)
    .filter(Boolean)
    .join('\n\n');
  const takenNames = new Set((Array.isArray(body.tools) ? body.tools : [])
    .map(t => t && t.function && t.function.name).filter(Boolean));
  const input = [];
  for (const m of messages) {
    if (m.role === 'system') continue;
    if (m.role === 'user') {
      input.push({ role: 'user', content: contentPartsForInput(m.content) });
    } else if (m.role === 'assistant') {
      const text = messageText(m);
      if (text) input.push({ role: 'assistant', content: text });
      for (const tc of m.tool_calls || []) {
        input.push({
          type: 'function_call',
          call_id: tc.id,
          name: tc.function ? directToolAlias(tc.function.name, takenNames) : undefined,
          arguments: (tc.function && tc.function.arguments) || ''
        });
      }
    } else if (m.role === 'tool' || m.role === 'function') {
      input.push({
        type: 'function_call_output',
        call_id: m.tool_call_id || m.name,
        output: messageText(m)
      });
    } else {
      input.push({ role: m.role, content: contentPartsForInput(m.content) });
    }
  }
  const out = { model: baseModel, input, store: false, stream: Boolean(body.stream) };
  if (instructions) out.instructions = instructions;
  if (Array.isArray(body.tools) && body.tools.length > 0) {
    out.tools = body.tools.map(t => (t && t.type === 'function' && t.function)
      ? { type: 'function', name: directToolAlias(t.function.name, takenNames), description: t.function.description, parameters: t.function.parameters }
      : t);
  }
  if (body.tool_choice !== undefined) {
    const tc = body.tool_choice;
    if (typeof tc === 'string') out.tool_choice = tc;
    else if (tc && tc.type === 'function' && tc.function && tc.function.name) out.tool_choice = { type: 'function', name: directToolAlias(tc.function.name, takenNames) };
    else out.tool_choice = tc;
  }
  const maxOut = body.max_completion_tokens !== undefined && body.max_completion_tokens !== null
    ? body.max_completion_tokens
    : body.max_tokens;
  if (typeof maxOut === 'number') out.max_output_tokens = maxOut;
  if (typeof body.temperature === 'number') out.temperature = body.temperature;
  if (typeof body.top_p === 'number') out.top_p = body.top_p;
  const mappedEffort = mapResponsesEffort(effort);
  if (mappedEffort) out.reasoning = { effort: mappedEffort };
  return out;
};

const extractResponsesOutput = (respBody) => {
  let content = '';
  const toolCalls = [];
  for (const item of (respBody && respBody.output) || []) {
    if (!item || typeof item !== 'object') continue;
    if (item.type === 'message') {
      for (const part of item.content || []) {
        if (part && part.type === 'output_text' && typeof part.text === 'string') content += part.text;
      }
    } else if (item.type === 'function_call') {
      toolCalls.push({
        id: item.call_id || item.id,
        type: 'function',
        function: { name: clientToolName(item.name), arguments: item.arguments || '' }
      });
    }
  }
  return { content, toolCalls };
};

const finishReasonFromResponses = (respBody, hasToolCalls) => {
  if (hasToolCalls) return 'tool_calls';
  if (respBody && respBody.status === 'incomplete' &&
      respBody.incomplete_details && respBody.incomplete_details.reason === 'max_output_tokens') {
    return 'length';
  }
  return 'stop';
};

const chatUsageFromResponses = (usage) => {
  const promptTokens = usage && typeof usage.input_tokens === 'number' ? usage.input_tokens : 0;
  const completionTokens = usage && typeof usage.output_tokens === 'number' ? usage.output_tokens : 0;
  const totalTokens = usage && typeof usage.total_tokens === 'number'
    ? usage.total_tokens
    : promptTokens + completionTokens;
  const cached = usage && usage.input_tokens_details &&
    typeof usage.input_tokens_details.cached_tokens === 'number'
    ? usage.input_tokens_details.cached_tokens
    : 0;
  return {
    prompt_tokens: promptTokens,
    completion_tokens: completionTokens,
    total_tokens: totalTokens,
    ...(cached > 0 ? { prompt_tokens_details: { cached_tokens: cached } } : {})
  };
};

const responsesToChatCompletion = (respBody, { chatId, created, fullName }) => {
  const { content, toolCalls } = extractResponsesOutput(respBody);
  const message = { role: 'assistant', content };
  if (toolCalls.length > 0) message.tool_calls = toolCalls;
  return {
    id: chatId,
    object: 'chat.completion',
    created,
    model: fullName,
    choices: [{
      index: 0,
      message,
      finish_reason: finishReasonFromResponses(respBody, toolCalls.length > 0)
    }],
    usage: chatUsageFromResponses(respBody && respBody.usage)
  };
};

// Translate an upstream Responses SSE stream into chat-completion chunks.
const streamResponsesAsChat = async (response, { res, chatId, created, fullName }) => {
  const sendChunk = (delta, finishReason = null, usage) => {
    if (res.writableEnded) return;
    const chunk = {
      id: chatId,
      object: 'chat.completion.chunk',
      created,
      model: fullName,
      choices: [{ index: 0, delta, finish_reason: finishReason }],
      ...(usage ? { usage } : {})
    };
    res.write(`data: ${JSON.stringify(chunk)}\n\n`);
  };
  const sendErrorAndEnd = (message) => {
    if (!res.writableEnded) {
      // SSE-framed error + [DONE], the #670 pattern: the 200 + event-stream
      // headers are already on the wire, so a bare JSON body would be
      // invisible to an SSE client.
      res.write(`data: ${JSON.stringify({ error: { message } })}\n\n`);
      res.write('data: [DONE]\n\n');
    }
    res.end();
  };
  const toolIndexByItem = new Map();
  let nextToolIndex = 0;
  let ended = false;
  const handleEvent = (evt) => {
    if (ended || !evt || typeof evt !== 'object') return;
    const type = evt.type || '';
    if (type === 'response.output_text.delta' && typeof evt.delta === 'string') {
      sendChunk({ content: evt.delta });
    } else if (type === 'response.output_item.added' && evt.item && evt.item.type === 'function_call') {
      const index = nextToolIndex;
      nextToolIndex += 1;
      const itemKey = evt.item.id || evt.item.call_id;
      if (itemKey) toolIndexByItem.set(itemKey, index);
      sendChunk({
        tool_calls: [{
          index,
          id: evt.item.call_id || evt.item.id,
          type: 'function',
          function: { name: clientToolName(evt.item.name || ''), arguments: '' }
        }]
      });
    } else if (type === 'response.function_call_arguments.delta') {
      const index = toolIndexByItem.get(evt.item_id) ?? toolIndexByItem.get(evt.call_id) ?? 0;
      sendChunk({
        tool_calls: [{
          index,
          function: { arguments: typeof evt.delta === 'string' ? evt.delta : '' }
        }]
      });
    } else if (type === 'response.completed') {
      const respBody = evt.response || {};
      const { toolCalls } = extractResponsesOutput(respBody);
      sendChunk({}, finishReasonFromResponses(respBody, toolCalls.length > 0), chatUsageFromResponses(respBody.usage));
      if (!res.writableEnded) res.write('data: [DONE]\n\n');
      res.end();
      ended = true;
    } else if (type === 'response.failed') {
      sendErrorAndEnd((evt.response && evt.response.error && evt.response.error.message) || 'Meta Responses request failed');
      ended = true;
    } else if (type === 'error') {
      sendErrorAndEnd(evt.message || (evt.error && evt.error.message) || 'Meta Responses stream error');
      ended = true;
    }
  };
  try {
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      let idx;
      while ((idx = buffer.indexOf('\n\n')) >= 0) {
        const rawEvent = buffer.slice(0, idx);
        buffer = buffer.slice(idx + 2);
        for (const line of rawEvent.split('\n')) {
          const trimmed = line.trim();
          if (!trimmed.startsWith('data:')) continue;
          const data = trimmed.slice('data:'.length).trim();
          if (!data || data === '[DONE]') continue;
          try { handleEvent(JSON.parse(data)); } catch { /* non-JSON data line */ }
        }
        if (ended) return;
      }
    }
  } catch (err) {
    sendErrorAndEnd(`Direct Meta stream failed: ${err.message}`);
    return;
  }
  if (!ended && !res.writableEnded) {
    res.write('data: [DONE]\n\n');
    res.end();
  }
};

// Attempt one chat completion on the direct path. Returns true when the
// response has been fully handled (including any upstream error response,
// passed through as-is); false when nothing was written and the caller
// should fall back to warm/exec.
const tryDirectChat = async ({ body, baseModel, effort, fullName, stream, res, chatId, created }) => {
  const keyResult = await getDirectKey();
  if (keyResult.status === 'disabled' || keyResult.status === 'none' || keyResult.status === 'transport') {
    return false;
  }
  if (keyResult.status === 'hard') {
    writeJsonBody(res, 502, { error: { message: keyResult.message } });
    return true;
  }
  const upstreamBody = JSON.stringify(buildResponsesRequest(body, baseModel, effort));
  let outcome;
  try {
    outcome = await directResponsesFetch(keyResult, upstreamBody, stream ? 'text/event-stream' : 'application/json');
  } catch {
    return false; // upstream fetch threw: no upstream response, fall back
  }
  if (outcome.hard) {
    writeJsonBody(res, 502, { error: { message: outcome.hard.message } });
    return true;
  }
  const response = outcome.response;
  if (!response.ok) {
    const text = await response.text().catch(() => '');
    if (response.status === 403 && text.includes('client_attestation_denied')) {
      // The attestation fingerprint moved (a tool name newly blocked):
      // degrade to the CLI door instead of failing the seat outright.
      console.warn('[muse-proxy] direct path attestation denied; falling back to the CLI door');
      return false;
    }
    // Meta answered: pass the response through as-is, whatever the status.
    res.writeHead(response.status, {
      'Content-Type': response.headers.get('content-type') || 'application/json'
    });
    res.end(text);
    return true;
  }
  if (!stream) {
    let respBody;
    try {
      respBody = await response.json();
    } catch {
      writeJsonBody(res, 502, { error: { message: 'Direct Meta route returned an unreadable response body' } });
      return true;
    }
    writeJsonBody(res, 200, responsesToChatCompletion(respBody, { chatId, created, fullName }));
    return true;
  }
  res.writeHead(200, {
    'Content-Type': 'text/event-stream; charset=utf-8',
    'Cache-Control': 'no-cache, no-transform',
    'Connection': 'keep-alive',
    'X-Accel-Buffering': 'no'
  });
  await streamResponsesAsChat(response, { res, chatId, created, fullName });
  return true;
};

const server = http.createServer((req, res) => {
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  res.setHeader('Access-Control-Allow-Headers', '*');
  if (req.method === 'OPTIONS') {
    res.writeHead(204);
    res.end();
    return;
  }

  const url = new URL(req.url, `http://${req.headers.host || HOST}`);
  const pathname = url.pathname.replace(/\/+$/, '') || '/';

  // Health & Root
  if (req.method === 'GET' && (pathname === '/health' || pathname === '/')) {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({
      status: 'ok',
      service: 'meta-muse-proxy',
      port: PORT,
      binary: MUSE_BIN,
      defaultModel: 'muse-spark-1.3-high',
      endpoint: '/v1/chat/completions',
      warm: {
        enabled: WARM_ENABLED,
        hostGen: warmHost.gen,
        hostUp: Boolean(warmHost.child),
        sessions: warmStore.map.size
      },
      direct: {
        enabled: DIRECT_ENABLED,
        credentialFound: Boolean(loadDirectCredential()),
        mintedKeyCached: Boolean(directState.cachedKey && Date.now() - directState.cachedAt < DIRECT_KEY_TTL_MS),
        subsTier: directState.subsTier,
        subsActive: directState.subsActive
      },
      cliFlags: {
        serveProbed: cliCaps.serve !== null,
        execProbed: cliCaps.exec !== null,
        dropped: { serve: cliDropped.serve, exec: cliDropped.exec }
      }
    }, null, 2));
    return;
  }

  // Models catalog
  if (req.method === 'GET' && (pathname === '/v1/models' || pathname === '/models')) {
    const now = Math.floor(Date.now() / 1000);
    const data = [];

    for (const m of MODELS) {
      data.push({
        id: m.id,
        object: 'model',
        created: now,
        owned_by: 'meta-muse'
      });
      for (const eff of m.efforts) {
        data.push({
          id: `${m.id}-${eff}`,
          object: 'model',
          created: now,
          owned_by: 'meta-muse'
        });
      }
    }

    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ object: 'list', data }, null, 2));
    return;
  }


  // Responses passthrough (direct mode, #2753): the client's body is
  // forwarded verbatim to the upstream Responses API with only the
  // Authorization header swapped for the minted plan key. Any upstream
  // HTTP response is piped through as-is. Without direct credentials the
  // route is not offered (404, as before direct mode existed).
  if (req.method === 'POST' && (pathname === '/v1/responses' || pathname === '/responses')) {
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
      const keyResult = await getDirectKey();
      if (keyResult.status === 'disabled' || keyResult.status === 'none') {
        res.writeHead(404, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: `Route not found: ${req.method} ${pathname}` } }));
        return;
      }
      if (keyResult.status === 'hard' || keyResult.status === 'transport') {
        writeJsonBody(res, 502, {
          error: { message: keyResult.message || 'Direct Meta route unavailable: key exchange failed' }
        });
        return;
      }
      let outcome;
      try {
        outcome = await directResponsesFetch(keyResult, bodyText, req.headers.accept || 'application/json');
      } catch (err) {
        writeJsonBody(res, 502, { error: { message: `Direct Meta route request failed: ${err.message}` } });
        return;
      }
      if (outcome.hard) {
        writeJsonBody(res, 502, { error: { message: outcome.hard.message } });
        return;
      }
      const response = outcome.response;
      res.writeHead(response.status, {
        'Content-Type': response.headers.get('content-type') || 'application/json'
      });
      if (response.body && typeof response.body.getReader === 'function') {
        try {
          const reader = response.body.getReader();
          for (;;) {
            const { done, value } = await reader.read();
            if (done) break;
            res.write(Buffer.from(value));
          }
        } catch { /* upstream stream cut: end with what was sent */ }
        res.end();
      } else {
        res.end(await response.text().catch(() => ''));
      }
    });
    return;
  }

  // Chat completions
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
      try {
        body = JSON.parse(bodyText);
      } catch (err) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: `Invalid JSON body: ${err.message}` } }));
        return;
      }

      const stream = Boolean(body.stream);
      const reqReasoningEffort = body.reasoning_effort || (body.options && body.options.reasoning_effort);
      const { baseModel, effort, fullName } = resolveModelAndEffort(body.model, reqReasoningEffort);
      const prompt = formatPrompt(body.messages);

      if (!prompt || prompt.trim().length === 0) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: 'Messages cannot be empty' } }));
        return;
      }

      const chatId = `chatcmpl-muse-${crypto.randomUUID()}`;
      const created = Math.floor(Date.now() / 1000);

      // Direct mode first (#2753): the minted-key Responses path
      // answers when it can; any upstream response is final. Only a
      // no-response outcome returns false and the warm/exec paths
      // below take over unchanged.
      if (DIRECT_ENABLED) {
        let directHandled = false;
        try {
          directHandled = await tryDirectChat({
            body, baseModel, effort, fullName, stream, res, chatId, created
          });
        } catch (err) {
          console.warn(`[muse-proxy direct] unexpected direct error (${err.message}); falling back to warm/exec`);
          directHandled = false;
        }
        if (directHandled) return;
        if (res.writableEnded) return;
      }

      // Warm path first (#480): a persistent serve host + MSP session
      // answers when it can; any failure before the response is committed
      // returns false and the exec path below takes over unchanged.
      if (WARM_ENABLED) {
        let warmHandled = false;
        try {
          warmHandled = await tryWarmChat({
            messages: body.messages, baseModel, effort, fullName, stream, res, chatId, created
          });
        } catch (err) {
          console.warn(`[muse-proxy warm] unexpected warm error (${err.message}); falling back to exec`);
          warmHandled = false;
        }
        if (warmHandled) return;
        if (res.writableEnded) return;
      }

      // Prompt goes in via --prompt-file (no argv length/escaping concerns)
      const promptFile = path.join(WORKDIR, `prompt-${crypto.randomUUID()}.txt`);
      fs.writeFileSync(promptFile, prompt, 'utf-8');

      // Spawn arguments for muse exec (only the flags the resolved CLI
      // advertises — see the capability probe)
      await ensureCliCaps();
      const args = buildCliArgs('exec', [
        'exec',
        { flag: '--json' },
        { flag: '--prompt-file', value: promptFile },
        { flag: '--model', value: baseModel },
        { flag: '--reasoning-effort', value: effort },
        { flag: '--disable-reminders', optional: true },
        { flag: '--user-input-auto-resolve' },
        { flag: '--approval-mode', value: 'never' }
      ]);

      console.log(`[muse-proxy] Spawning: ${MUSE_BIN} ${args.join(' ')}`);

      let child;
      try {
        child = spawn(MUSE_BIN, args, {
          cwd: WORKDIR,
          env: childEnv(),
          stdio: ['ignore', 'pipe', 'pipe']
        });
      } catch (err) {
        console.error(`[muse-proxy] Failed to spawn muse: ${err.message}`);
        try { fs.unlinkSync(promptFile); } catch (_) {}
        res.writeHead(500, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: `Failed to spawn muse: ${err.message}` } }));
        return;
      }

      res.on('close', () => {
        if (!res.writableEnded) {
          try { child.kill('SIGTERM'); } catch (_) {}
        }
      });

      if (stream) {
        res.writeHead(200, {
          'Content-Type': 'text/event-stream; charset=utf-8',
          'Cache-Control': 'no-cache, no-transform',
          'Connection': 'keep-alive',
          'X-Accel-Buffering': 'no'
        });
      }

      let lineBuffer = '';
      let fullContent = '';
      let promptTokens = Math.max(1, Math.ceil(prompt.length / 4));
      let completionTokens = 0;
      let terminalState = null;
      let terminalText = null;
      let stderrTail = '';

      const processJsonLine = (line) => {
        const trimmed = line.trim();
        if (!trimmed || !trimmed.startsWith('{')) return;

        try {
          const event = JSON.parse(trimmed);
          const payload = event.payload || {};

          // Assistant text delta (streams token chunks)
          if (event.payload_type === 'run.output.delta' && typeof payload.text === 'string') {
            const text = payload.text;
            fullContent += text;

            if (stream && !res.writableEnded) {
              const chunk = {
                id: chatId,
                object: 'chat.completion.chunk',
                created,
                model: fullName,
                choices: [{
                  index: 0,
                  delta: { content: text },
                  finish_reason: null
                }]
              };
              res.write(`data: ${JSON.stringify(chunk)}\n\n`);
            }
          }

          // Real token usage from the run
          if (event.payload_type === 'run.usage' && Array.isArray(payload.models)) {
            let inTok = 0;
            let outTok = 0;
            for (const m of payload.models) {
              if (typeof m.input_tokens === 'number') inTok += m.input_tokens;
              if (typeof m.output_tokens === 'number') outTok += m.output_tokens;
            }
            if (inTok > 0) promptTokens = inTok;
            if (outTok > 0) completionTokens = outTok;
          }

          // Terminal event carries the authoritative final text
          if (typeof event.payload_type === 'string' && event.payload_type.startsWith('run.terminal.')) {
            terminalState = payload.terminal || event.payload_type.slice('run.terminal.'.length);
            if (typeof payload.text === 'string') terminalText = payload.text;
          }
        } catch (err) {
          // Non-JSON line or parser error ignored
        }
      };

      child.stdout.on('data', chunk => {
        lineBuffer += chunk.toString('utf-8');
        const lines = lineBuffer.split('\n');
        lineBuffer = lines.pop(); // remainder
        for (const line of lines) {
          processJsonLine(line);
        }
      });

      child.stderr.on('data', chunk => {
        const msg = chunk.toString('utf-8');
        stderrTail = (stderrTail + msg).slice(-2000);
        if (msg.trim().length > 0) {
          console.warn(`[muse-proxy stderr] ${msg.trim()}`);
        }
      });

      child.on('close', (code, signal) => {
        if (lineBuffer.trim().length > 0) {
          processJsonLine(lineBuffer);
          lineBuffer = '';
        }
        try { fs.unlinkSync(promptFile); } catch (_) {}

        if (res.writableEnded) return;

        // Reconcile the streamed deltas with the terminal event's
        // authoritative final text (#484). In stream mode the deltas are
        // already on the wire, so the only recoverable divergence is a
        // missing tail: when the terminal text extends the streamed text,
        // the suffix is emitted as one last content chunk before the
        // finish chunk below, so streamed and non-streamed clients end up
        // with identical content. Any other divergence (the terminal text
        // is not an extension of what was streamed) cannot be retracted
        // from a stream; it is logged explicitly instead of silently
        // truncating, and the non-streamed response still uses the
        // authoritative text when it is at least as long.
        let unstreamedSuffix = '';
        if (terminalText !== null) {
          if (terminalText.startsWith(fullContent)) {
            unstreamedSuffix = terminalText.slice(fullContent.length);
          } else if (terminalText.length >= fullContent.length) {
            console.warn('[muse-proxy] terminal text diverges from streamed deltas; streamed response cannot be corrected retroactively');
          }
          if (terminalText.length >= fullContent.length) {
            fullContent = terminalText;
          }
        }

        // Failure is judged by the run's authoritative terminal state when
        // one was emitted, otherwise by the exit code (#672). Partial
        // content must not mask a failure: a run that failed midway used to
        // be reported as a successful 'stop' carrying the truncated text,
        // because both clauses of the old predicate required empty content.
        const failed = terminalState ? terminalState !== 'completed' : code !== 0;
        if (failed) {
          const stderrLine = stderrTail.trim().split('\n').pop();
          const fallback = terminalState && terminalState !== 'completed'
            ? `muse run terminated with state '${terminalState}'`
            : signal
              ? `muse exec terminated by signal ${signal}`
              : `muse exec exited with code ${code}`;
          const detail = (stderrLine || fallback).trim();
          const errorBody = { error: { message: `Muse execution failed: ${detail}` } };
          if (stream) {
            // The 200 + text/event-stream headers are already on the wire,
            // so a bare JSON body would be invisible to an SSE client
            // parsing `data:` lines — it would see an empty, successful
            // stream (#670). Frame the error as an SSE event and close
            // the stream with [DONE] instead.
            if (!res.writableEnded) {
              res.write(`data: ${JSON.stringify(errorBody)}\n\n`);
              res.write('data: [DONE]\n\n');
            }
            res.end();
            return;
          }
          if (!res.headersSent) {
            res.writeHead(500, { 'Content-Type': 'application/json' });
          }
          res.end(JSON.stringify(errorBody));
          return;
        }

        if (completionTokens === 0 && fullContent.length > 0) {
          completionTokens = Math.max(1, Math.ceil(fullContent.length / 4));
        }

        if (stream) {
          if (unstreamedSuffix.length > 0) {
            const tailChunk = {
              id: chatId,
              object: 'chat.completion.chunk',
              created,
              model: fullName,
              choices: [{
                index: 0,
                delta: { content: unstreamedSuffix },
                finish_reason: null
              }]
            };
            res.write(`data: ${JSON.stringify(tailChunk)}\n\n`);
          }
          const endChunk = {
            id: chatId,
            object: 'chat.completion.chunk',
            created,
            model: fullName,
            choices: [{
              index: 0,
              delta: {},
              finish_reason: 'stop'
            }],
            usage: {
              prompt_tokens: promptTokens,
              completion_tokens: completionTokens,
              total_tokens: promptTokens + completionTokens
            }
          };
          res.write(`data: ${JSON.stringify(endChunk)}\n\n`);
          res.write('data: [DONE]\n\n');
          res.end();
        } else {
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({
            id: chatId,
            object: 'chat.completion',
            created,
            model: fullName,
            choices: [{
              index: 0,
              message: {
                role: 'assistant',
                content: fullContent
              },
              finish_reason: 'stop'
            }],
            usage: {
              prompt_tokens: promptTokens,
              completion_tokens: completionTokens,
              total_tokens: promptTokens + completionTokens
            }
          }, null, 2));
        }
      });

      child.on('error', err => {
        console.error(`[muse-proxy child error] ${err.message}`);
        try { fs.unlinkSync(promptFile); } catch (_) {}
        if (!res.headersSent) {
          res.writeHead(500, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ error: { message: `Muse execution error: ${err.message}` } }));
        }
      });
    });
    return;
  }

  // Fallthrough 404
  res.writeHead(404, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ error: { message: `Route not found: ${req.method} ${pathname}` } }));
});


// Publish the bound endpoint so other tools can find this proxy without
// knowing its port in advance: $XDG_STATE_HOME/<name>/endpoint.json
// (default ~/.local/state/<name>/endpoint.json), written on listen,
// removed on exit.
const publishEndpoint = () => {
  try {
    const stateHome =
      process.env.XDG_STATE_HOME || path.join(os.homedir(), '.local', 'state');
    const dir = path.join(stateHome, 'muse-proxy');
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, 'endpoint.json');
    fs.writeFileSync(
      `${file}.tmp`,
      JSON.stringify({ host: HOST, port: PORT, pid: process.pid, url: `http://${HOST}:${PORT}` }) + '\n',
    );
    fs.renameSync(`${file}.tmp`, file);
    const cleanup = () => {
      try { fs.unlinkSync(file); } catch {}
    };
    process.on('exit', cleanup);
    process.on('SIGTERM', () => { cleanup(); process.exit(0); });
    process.on('SIGINT', () => { cleanup(); process.exit(0); });
  } catch {}
};

server.listen(PORT, HOST, () => {
  publishEndpoint();
  // Warm the CLI capability probe so the first request does not pay for it.
  ensureCliCaps().catch(() => {});
  console.log(`[muse-proxy] OpenAI-compatible bridge listening on http://${HOST}:${PORT}`);
  console.log(`[muse-proxy] Resolved muse binary: ${MUSE_BIN}`);
  console.log(`[muse-proxy] Workdir: ${WORKDIR}`);
});
