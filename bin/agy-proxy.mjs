#!/usr/bin/env node
// agy-proxy — OpenAI-compatible bridge to the Antigravity CLI (`agy`).
//
// Serving path (2026-10-28): RESIDENT warm `agy` processes speaking
// `--input-format stream-json` / `--output-format stream-json` (agy's
// documented warm shape: one process holds one continuous conversation,
// one NDJSON `{"event":"user",...}` line per turn, one `result` event
// per turn). This replaces the previous spawn-per-request `-p` path,
// where every request paid full CLI + language-server startup.
//
// Worker lifecycle:
//   - Session requests (explicit id / `user:` / first-message hash, see
//     getSessionKey) own a dedicated resident worker keyed by
//     (model, session key). Later turns on that worker send only the
//     newest user message; the worker's conversation holds the history.
//     If no live worker exists but a conversation id is known, a worker
//     is spawned with `--conversation <id>` to resume it.
//   - Stateless requests claim a PRE-WARMED spare worker (spawned ahead
//     of time, init already received) for exactly one fresh-conversation
//     turn, then retire it — a used worker's context is never handed to
//     an unrelated request — and a replacement spare is spawned in the
//     background. With no spare ready, a worker is spawned on demand.
//   - A worker that dies, errors its turn (result status != SUCCESS),
//     or times out is killed and never reused; the next request respawns
//     (spares replenish automatically). Live session workers are evicted
//     after an idle TTL / past a live-worker cap.
// Cached-token accounting (#480, settled 2026-11-01): the agy CLI's
// stream-json `result.usage` DOES carry a cached-input figure —
// `cache_read_tokens`, cumulative over the session like its sibling
// counters (verified against the real CLI's result shape on the
// laptop: usage keys are input_tokens, output_tokens,
// thinking_tokens, cache_read_tokens, total_tokens). The per-turn
// delta is emitted as `prompt_tokens_details.cached_tokens` in both
// response shapes when > 0, matching the sibling proxies'
// opencode#394 contract; when the CLI reports none, the field is
// omitted entirely rather than reporting a fabricated zero split.
// Failure semantics are unchanged from #678: a failed turn is an error
// (502 non-stream, SSE error event + [DONE] stream), never a 'stop'
// carrying partial output; a failed resume retries once as a fresh
// conversation with the full prompt when nothing was output yet.
import http from 'node:http';
import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { parseProxyPort } from './proxy-port.mjs';

const PORT = parseProxyPort('AGY_PROXY_PORT', 18743);
const HOST = process.env.AGY_PROXY_HOST || '127.0.0.1';

// Publish the bound endpoint so other tools can find this proxy without
// knowing its port in advance: $XDG_STATE_HOME/<name>/endpoint.json
// (default ~/.local/state/<name>/endpoint.json), written on listen,
// removed on exit.
const publishEndpoint = () => {
  try {
    const stateHome =
      process.env.XDG_STATE_HOME || path.join(os.homedir(), '.local', 'state');
    const dir = path.join(stateHome, 'agy-proxy');
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, 'endpoint.json');
    fs.writeFileSync(
      `${file}.tmp`,
      JSON.stringify({ host: HOST, port: PORT, pid: process.pid, url: `http://${HOST}:${PORT}` }) + '\n',
    );
    fs.renameSync(`${file}.tmp`, file);
    const cleanup = () => {
      try {
        fs.unlinkSync(file);
      } catch {}
    };
    process.on('exit', cleanup);
    process.on('SIGTERM', () => {
      cleanup();
      process.exit(0);
    });
    process.on('SIGINT', () => {
      cleanup();
      process.exit(0);
    });
  } catch {}
};

// The served model set is pulled LIVE from `agy models` (Drew's
// fleet-wide rule, 2026-10-28: providers pull their model list live
// unless not possible), in this proxy's own service context — the
// pull spawns `agy models` with the same child environment the
// workers get (HOME=AGY_HOME), which is the context whose auth works.
// The list is cached in memory for MODELS_TTL_MS (stale-while-
// revalidate: requests never block on a refresh after the first
// load), refreshed in the background when stale and immediately
// (async) when a request names an unknown model. Last-known-good
// fallback chain when a pull fails: the on-disk cache of the last
// successful pull ($XDG_STATE_HOME/agy-proxy/models-cache.json),
// then FALLBACK_MODELS — the list as pulled on 2026-10-28 (agy
// 1.2.16: 13 ids over 5 bases), kept only as a cold-start floor.
// Unknown ids (against the current live / last-known-good list)
// return HTTP 400, never a silent fallback (#1692).
const FALLBACK_MODELS = [
  { id: 'gemini-4-argon', name: 'Gemini 4 Argon (agy)', efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' },
  { id: 'gemini-3.8-flash', name: 'Gemini 3.8 Flash (agy)', efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' },
  { id: 'claude-sonnet-5-5', name: 'Claude Sonnet 5.5 (agy)', efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' },
  { id: 'claude-opus-5-5', name: 'Claude Opus 5.5 (agy)', efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' },
  { id: 'gpt-oss-120b', name: 'GPT-OSS 120B (agy)', efforts: ['medium'], defaultEffort: 'medium' }
];

const findAgyBinary = () => {
  // Explicit override first: the test suite points this at its stub so
  // a host-installed agy can never shadow it (#3355, mirroring
  // CODEX_PROXY_CODEX_BIN in codex-proxy.mjs, #694).
  if (process.env.AGY_PROXY_AGY_BIN) return process.env.AGY_PROXY_AGY_BIN;
  const candidates = [
    '/run/current-system/sw/bin/agy',
    `${process.env.HOME}/.gemini/bin/agy`,
    `${process.env.HOME}/.local/bin/agy`,
    'agy'
  ];
  for (const c of candidates) {
    if (c === 'agy' || fs.existsSync(c)) {
      return c;
    }
  }
  return 'agy';
};

const AGY_BIN = findAgyBinary();

const REAL_HOME = process.env.HOME || '/home/user';
const AGY_HOME = process.env.AGY_ISOLATED_HOME || `${REAL_HOME}/.gemini/agy-proxy-env`;

const ensureAgyEnvironment = () => {
  const geminiDir = `${AGY_HOME}/.gemini`;
  const configDir = `${geminiDir}/config`;
  try {
    fs.mkdirSync(configDir, { recursive: true });

    const authFiles = [
      'oauth_creds.json',
      'gemini-credentials.json',
      'google_accounts.json',
      'installation_id',
      'projects.json',
      'settings.json',
      'state.json',
      'trustedFolders.json'
    ];

    for (const file of authFiles) {
      const src = `${REAL_HOME}/.gemini/${file}`;
      const dst = `${geminiDir}/${file}`;
      if (fs.existsSync(src)) {
        try {
          if (fs.existsSync(dst)) fs.unlinkSync(dst);
          fs.symlinkSync(src, dst);
        } catch {}
      }
    }

    const mcpConfigPath = `${configDir}/mcp_config.json`;
    fs.writeFileSync(mcpConfigPath, JSON.stringify({ mcpServers: {} }, null, 2));

    const userConfigPath = `${configDir}/config.json`;
    fs.writeFileSync(userConfigPath, JSON.stringify({
      plugins: {},
      userSettings: {
        browserJsExecutionPolicy: 'BROWSER_JS_EXECUTION_POLICY_DISABLED',
        enableTerminalSandbox: false
      }
    }, null, 2));
  } catch (err) {
    console.error('Warning: failed to initialize isolated agy environment:', err);
  }
  return AGY_HOME;
};

ensureAgyEnvironment();

// ---------------------------------------------------------------------------
// Live model catalog (see the FALLBACK_MODELS comment above)
// ---------------------------------------------------------------------------
const MODELS_TTL_MS = Number(process.env.AGY_PROXY_MODELS_TTL_MS || 300000);
const MODELS_PULL_TIMEOUT_MS = 30000;
const modelsCacheFile = () => path.join(
  process.env.XDG_STATE_HOME || path.join(os.homedir(), '.local', 'state'),
  'agy-proxy', 'models-cache.json'
);

let modelCatalog = FALLBACK_MODELS;
let modelsSource = 'fallback'; // 'live' | 'cache' | 'fallback'
let modelsFetchedAt = 0;
let modelsPullInFlight = null;

// Group `agy models` full ids (`<base>-<effort>` TSV lines) into the
// base-model catalog shape resolveModelAndEffort / /v1/models use.
const catalogFromFullIds = (fullIds) => {
  const bases = new Map();
  for (const { id, name } of fullIds) {
    let base = id;
    let effort = null;
    for (const suffix of ['-low', '-medium', '-high']) {
      if (id.endsWith(suffix)) {
        effort = suffix.slice(1);
        base = id.slice(0, -suffix.length);
        break;
      }
    }
    if (!bases.has(base)) {
      const baseName = (name || base).replace(/\s*\((Low|Medium|High)\)\s*$/, '');
      bases.set(base, { id: base, name: `${baseName} (agy)`, efforts: [], defaultEffort: null });
    }
    const entry = bases.get(base);
    if (effort && !entry.efforts.includes(effort)) entry.efforts.push(effort);
  }
  const catalog = [...bases.values()].filter(m => m.efforts.length > 0);
  for (const m of catalog) {
    m.defaultEffort = m.efforts.includes('medium') ? 'medium' : m.efforts[0];
  }
  return catalog;
};

const parseAgyModelsOutput = (stdout) => {
  const fullIds = [];
  for (const line of stdout.split('\n')) {
    const [id, name] = line.split('\t');
    if (!id || !/^[a-z0-9][a-z0-9._-]*$/i.test(id.trim())) continue;
    fullIds.push({ id: id.trim(), name: (name || '').trim() });
  }
  return fullIds;
};

const persistModelsCache = (fullIds) => {
  try {
    const file = modelsCacheFile();
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(`${file}.tmp`, JSON.stringify({ fetchedAt: Date.now(), models: fullIds }, null, 2) + '\n');
    fs.renameSync(`${file}.tmp`, file);
  } catch {}
};

const loadModelsCache = () => {
  try {
    const cached = JSON.parse(fs.readFileSync(modelsCacheFile(), 'utf8'));
    if (Array.isArray(cached.models) && cached.models.length > 0) {
      const catalog = catalogFromFullIds(cached.models);
      if (catalog.length > 0) {
        modelCatalog = catalog;
        modelsSource = 'cache';
        modelsFetchedAt = cached.fetchedAt || 0;
        console.log(`[agy-proxy] Model catalog from last-known-good cache (${catalog.length} bases, pulled ${new Date(modelsFetchedAt).toISOString()})`);
      }
    }
  } catch { /* no cache yet */ }
};

const pullModelsLive = () => new Promise((resolve) => {
  const child = spawn(AGY_BIN, ['models'], {
    cwd: '/tmp',
    env: { ...process.env, HOME: AGY_HOME },
    stdio: ['ignore', 'pipe', 'pipe']
  });
  let stdout = '';
  let settled = false;
  const done = (ok, fullIds) => {
    if (settled) return;
    settled = true;
    resolve(ok ? fullIds : null);
  };
  const timer = setTimeout(() => { try { child.kill('SIGKILL'); } catch {} done(false); }, MODELS_PULL_TIMEOUT_MS);
  child.stdout.on('data', d => { stdout += d; });
  child.stderr.on('data', () => {});
  child.on('error', () => { clearTimeout(timer); done(false); });
  child.on('close', (code) => {
    clearTimeout(timer);
    if (code !== 0) return done(false);
    const fullIds = parseAgyModelsOutput(stdout);
    done(fullIds.length > 0, fullIds);
  });
});

const refreshModels = () => {
  if (modelsPullInFlight) return modelsPullInFlight;
  modelsPullInFlight = (async () => {
    const fullIds = await pullModelsLive();
    if (fullIds) {
      const catalog = catalogFromFullIds(fullIds);
      if (catalog.length > 0) {
        modelCatalog = catalog;
        modelsSource = 'live';
        modelsFetchedAt = Date.now();
        persistModelsCache(fullIds);
        console.log(`[agy-proxy] Model catalog pulled live from \`agy models\` (${catalog.length} bases, ${fullIds.length} ids)`);
      }
    } else {
      console.warn('[agy-proxy] Live `agy models` pull failed; serving last-known-good catalog');
    }
    modelsPullInFlight = null;
  })();
  return modelsPullInFlight;
};

// Serve-stale refresh check, run on catalog reads.
const maybeRefreshModels = () => {
  if (Date.now() - modelsFetchedAt > MODELS_TTL_MS) refreshModels();
};

// Requests block only on the very first load (no cache, no pull yet),
// and only up to the pull timeout; afterwards the catalog is always
// served from memory while refreshes happen in the background.
let initialModelsPull = null;
const ensureInitialModels = async () => {
  if (modelsFetchedAt > 0) return;
  if (!initialModelsPull) initialModelsPull = refreshModels();
  await Promise.race([
    initialModelsPull,
    new Promise(r => setTimeout(r, MODELS_PULL_TIMEOUT_MS + 5000))
  ]);
};

loadModelsCache();
refreshModels();

const modelsStatus = () => ({
  source: modelsSource,
  bases: modelCatalog.length,
  ids: modelCatalog.reduce((n, m) => n + 1 + m.efforts.length, 0),
  fetchedAt: modelsFetchedAt ? new Date(modelsFetchedAt).toISOString() : null,
  ttlMs: MODELS_TTL_MS
});

const resolveModelAndEffort = (requestedModel, requestedEffort) => {
  let modelStr = (requestedModel || 'gemini-3.8-flash').toLowerCase().trim();
  let effort = requestedEffort ? requestedEffort.toLowerCase().trim() : null;

  // Check if model string ends with -low, -medium, or -high
  for (const suffix of ['-low', '-medium', '-high']) {
    if (modelStr.endsWith(suffix)) {
      effort = suffix.slice(1);
      modelStr = modelStr.slice(0, -suffix.length);
      break;
    }
  }

  // Find base model config (in the live / last-known-good catalog)
  const modelConfig = modelCatalog.find(m => m.id === modelStr || m.id.replace(/-/g, '') === modelStr.replace(/-/g, ''));
  if (!modelConfig) return null;

  // Resolve effort
  if (!effort || !modelConfig.efforts.includes(effort)) {
    effort = modelConfig.defaultEffort;
  }

  return {
    baseModel: modelConfig.id,
    effort,
    fullName: `${modelConfig.id}-${effort}`
  };
};

// In-memory session store mapping sessionKey -> { conversationId, updatedAt }
const sessionStore = new Map();
const MAX_SESSIONS = 1000;

const pruneSessionStore = () => {
  if (sessionStore.size <= MAX_SESSIONS) return;
  const entries = Array.from(sessionStore.entries())
    .sort((a, b) => a[1].updatedAt - b[1].updatedAt);
  const toDelete = entries.slice(0, entries.length - MAX_SESSIONS);
  for (const [key] of toDelete) {
    sessionStore.delete(key);
  }
};

const extractLatestUserPrompt = (messages) => {
  if (!Array.isArray(messages) || messages.length === 0) return '';
  const lastUserMsg = [...messages].reverse().find(m => m.role === 'user');
  if (lastUserMsg) {
    if (typeof lastUserMsg.content === 'string') return lastUserMsg.content;
    if (Array.isArray(lastUserMsg.content)) {
      return lastUserMsg.content.map(c => c.text || JSON.stringify(c)).join('\n');
    }
    return JSON.stringify(lastUserMsg.content);
  }
  return formatPrompt(messages);
};

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
    return { key: explicitId.trim(), explicit: true, conversationId: explicitId.trim() };
  }

  if (body.user && typeof body.user === 'string' && body.user.trim()) {
    const userKey = `user:${body.user.trim()}`;
    const cached = sessionStore.get(userKey);
    return { key: userKey, explicit: false, conversationId: cached?.conversationId };
  }

  // Multi-turn automatic session tracking
  if (Array.isArray(body.messages) && body.messages.length > 1) {
    const firstMsg = body.messages[0];
    const firstContent = typeof firstMsg?.content === 'string'
      ? firstMsg.content
      : JSON.stringify(firstMsg?.content || '');
    if (firstContent.length > 0) {
      const hash = crypto.createHash('sha256').update(firstContent).digest('hex').slice(0, 16);
      const sessionKey = `turn0:${hash}`;
      const cached = sessionStore.get(sessionKey);
      return { key: sessionKey, explicit: false, conversationId: cached?.conversationId };
    }
  }

  return { key: null, explicit: false, conversationId: null };
};

const formatPrompt = (messages) => {
  if (!Array.isArray(messages) || messages.length === 0) return '';
  if (messages.length === 1 && messages[0].role === 'user') {
    return typeof messages[0].content === 'string'
      ? messages[0].content
      : JSON.stringify(messages[0].content);
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
      const role = m.role === 'assistant' ? 'Assistant' : (m.role === 'user' ? 'User' : m.role);
      let text = '';
      if (typeof m.content === 'string') {
        text = m.content;
      } else if (Array.isArray(m.content)) {
        text = m.content.map(c => c.text || JSON.stringify(c)).join('\n');
      } else {
        text = JSON.stringify(m.content);
      }
      return `${role}: ${text}`;
    }).join('\n\n');
    parts.push(convoText);
  }

  return parts.join('\n\n');
};

// ---------------------------------------------------------------------------
// Resident warm `agy` stream-json workers
// ---------------------------------------------------------------------------
const TURN_TIMEOUT_MS = Number(process.env.AGY_PROXY_TURN_TIMEOUT_MS || 300000);
const INIT_TIMEOUT_MS = 60000;
const SPARES_ENABLED = process.env.AGY_PROXY_WARM_SPARES !== '0';
const MAX_LIVE_WORKERS = Number(process.env.AGY_PROXY_MAX_LIVE_WORKERS || 32);
const SESSION_IDLE_TTL_MS = Number(process.env.AGY_PROXY_SESSION_TTL_MS || 30 * 60 * 1000);
// Models pre-warmed at startup (one spare each, default effort). Every
// other model warms its first spare after its first completed turn.
const PREWARM_MODELS = (process.env.AGY_PROXY_PREWARM_MODELS || 'gemini-4-argon-medium,gemini-3.8-flash-medium')
  .split(',').map(s => s.trim()).filter(Boolean);

const turnError = (detail, props = {}) => {
  const err = new Error(detail);
  err.detail = detail;
  err.hasOutput = false;
  Object.assign(err, props);
  return err;
};

class AgyWorker {
  constructor({ fullName, conversationId = null }) {
    this.fullName = fullName;
    this.conversationId = conversationId || null;
    this.spawnedAt = Date.now();
    this.lastUsedAt = Date.now();
    this.turns = 0;
    this.dead = null; // Error once the process is gone
    this.lineBuffer = '';
    this.stderrTail = '';
    this.turnWaiter = null;
    this.ready = false; // set when the init event arrives
    this.prevUsage = { input_tokens: 0, output_tokens: 0, total_tokens: 0, cache_read_tokens: 0 };

    const args = [
      '--dangerously-skip-permissions',
      '--disable-slash-commands'
    ];
    if (conversationId) {
      args.push('--conversation', conversationId);
    }
    // No `-p`: in stream-json input mode prompts arrive on stdin, one
    // NDJSON user event per turn (a `-p` prompt would be dropped).
    args.push(
      '--model', fullName,
      '--input-format', 'stream-json',
      '--output-format', 'stream-json'
    );

    const child = spawn(AGY_BIN, args, {
      cwd: '/tmp',
      env: {
        ...process.env,
        HOME: AGY_HOME
      },
      stdio: ['pipe', 'pipe', 'pipe']
    });
    this.child = child;
    this.pid = child.pid;

    this.initPromise = new Promise((resolve, reject) => {
      this._initResolve = resolve;
      this._initReject = reject;
      setTimeout(() => {
        if (!this.dead && !this.ready) {
          this._fail(turnError(`agy session did not init within ${INIT_TIMEOUT_MS}ms`));
          this.kill();
        }
      }, INIT_TIMEOUT_MS).unref();
    });
    this.initPromise.catch(() => {}); // surfaced via runTurn

    child.stdout.on('data', chunk => this._onStdout(chunk));
    child.stderr.on('data', chunk => {
      this.stderrTail = (this.stderrTail + chunk.toString()).slice(-4000);
    });
    child.on('error', err => {
      this._fail(turnError(err.message, { isSpawnError: true }));
    });
    child.on('close', (code, signal) => {
      // Flush any unterminated final line before judging the exit
      // (#683): a partial delta that never got its newline is real
      // output, and the resume-retry gate reads hasOutput from the
      // deltas this flush delivers.
      this._flushLineBuffer();
      // Failure is judged by the child's exit (#678): the last stderr
      // line names the cause when there is one, then the signal, then
      // the exit code — the same detail ladder the cold path used.
      const stderrLine = this.stderrTail.trim().split('\n').filter(Boolean).pop();
      const detail = stderrLine
        || (signal ? `agy process terminated by signal ${signal}` : `agy process exited with code ${code}`);
      this._fail(turnError(detail, { isExitError: true, code, signal, stderr: this.stderrTail }));
    });
  }

  _fail(err) {
    if (this.dead) return;
    this.dead = err;
    if (this._initReject) this._initReject(err);
    if (this.turnWaiter) {
      const w = this.turnWaiter;
      this.turnWaiter = null;
      err.hasOutput = w.deltas.length > 0;
      w.reject(err);
    }
  }

  _onStdout(chunk) {
    this.lineBuffer += chunk.toString();
    const lines = this.lineBuffer.split('\n');
    this.lineBuffer = lines.pop() || '';
    for (const line of lines) this._processLine(line);
  }

  _processLine(line) {
    if (!line.trim()) return;
    let obj;
    try { obj = JSON.parse(line); } catch { return; }
    this._onEvent(obj);
  }

  // The child can exit mid-line: whatever tail is still buffered is a
  // complete final line for judging purposes (#683) -- run it through
  // the same path a terminated line takes before any close judgment.
  _flushLineBuffer() {
    const tail = this.lineBuffer;
    this.lineBuffer = '';
    if (tail.trim()) this._processLine(tail);
  }

  _onEvent(obj) {
    if (obj.conversation_id) this.conversationId = obj.conversation_id;
    if (obj.event === 'init') {
      this.ready = true;
      if (obj.conversation_id) this.conversationId = obj.conversation_id;
      if (this._initResolve) this._initResolve(this.conversationId);
      return;
    }
    const w = this.turnWaiter;
    if (!w) return;
    if (obj.event === 'step_update' && obj.step_update?.text_delta) {
      const delta = obj.step_update.text_delta;
      w.deltas += delta;
      if (w.onDelta) { try { w.onDelta(delta); } catch {} }
      return;
    }
    if (obj.event === 'result') {
      this.turnWaiter = null;
      const result = obj.result || {};
      if (result.conversation_id) this.conversationId = result.conversation_id;
      if (result.status && result.status !== 'SUCCESS') {
        // A turn-level error (agy docs: the result carries `error`).
        // The session's state is unknown afterwards — the caller kills
        // this worker rather than reusing it.
        const detail = (typeof result.error === 'string' && result.error.trim())
          || `agy turn ended with status ${result.status}`;
        const err = turnError(detail, { isResultError: true, stderr: this.stderrTail });
        err.hasOutput = w.deltas.length > 0;
        w.reject(err);
        return;
      }
      // `usage` on a stream-json result is CUMULATIVE over the session
      // (agy docs); the per-turn usage is the delta from the previous
      // turn's cumulative counters.
      const cum = result.usage || {};
      const deltaUsage = {
        input_tokens: Math.max(0, (cum.input_tokens || 0) - this.prevUsage.input_tokens),
        output_tokens: Math.max(0, (cum.output_tokens || 0) - this.prevUsage.output_tokens),
        total_tokens: Math.max(0, (cum.total_tokens || 0) - this.prevUsage.total_tokens),
        cache_read_tokens: Math.max(0, (cum.cache_read_tokens || 0) - this.prevUsage.cache_read_tokens)
      };
      this.prevUsage = {
        input_tokens: cum.input_tokens || 0,
        output_tokens: cum.output_tokens || 0,
        total_tokens: cum.total_tokens || 0,
        cache_read_tokens: cum.cache_read_tokens || 0
      };
      this.turns += 1;
      this.lastUsedAt = Date.now();
      w.resolve({
        text: typeof result.response === 'string' ? result.response : w.deltas,
        deltas: w.deltas,
        usage: deltaUsage,
        conversationId: this.conversationId
      });
    }
  }

  async runTurn(promptText, onDelta) {
    if (this.dead) { const e = this.dead; e.hasOutput = false; throw e; }
    await this.initPromise;
    if (this.dead) { const e = this.dead; e.hasOutput = false; throw e; }
    if (this.turnWaiter) throw turnError('agy worker already has a turn in flight');
    return new Promise((resolve, reject) => {
      const waiter = {
        deltas: '',
        onDelta,
        resolve: (v) => { clearTimeout(timer); resolve(v); },
        reject: (e) => { clearTimeout(timer); reject(e); }
      };
      const timer = setTimeout(() => {
        if (this.turnWaiter === waiter) {
          this.turnWaiter = null;
          this.kill();
          const err = turnError(`agy turn timed out after ${TURN_TIMEOUT_MS}ms`, { isTimeout: true });
          err.hasOutput = waiter.deltas.length > 0;
          waiter.reject(err);
        }
      }, TURN_TIMEOUT_MS);
      this.turnWaiter = waiter;
      try {
        this.child.stdin.write(JSON.stringify({ event: 'user', message: { content: promptText } }) + '\n');
      } catch (err) {
        this.turnWaiter = null;
        waiter.reject(turnError(err.message, { isSpawnError: true }));
      }
    });
  }

  kill() {
    // Settle any in-flight turn / pending init first, so a killed
    // worker never leaves a request hanging until the turn timeout.
    this._fail(turnError('agy worker killed'));
    if (this.child && !this.child.killed) {
      try { this.child.kill('SIGTERM'); } catch {}
      const child = this.child;
      setTimeout(() => { try { child.kill('SIGKILL'); } catch {} }, 3000).unref();
    }
  }
}

// Live session workers, keyed `${fullName}|${sessionKey}`.
const liveWorkers = new Map();
// One pre-warmed spare per model fullName: spawned ahead of a request,
// init already received, zero turns taken — its conversation is fresh.
const spares = new Map();

const ensureSpare = (fullName) => {
  if (!SPARES_ENABLED) return;
  const existing = spares.get(fullName);
  if (existing && !existing.dead) return;
  if (existing) spares.delete(fullName);
  const w = new AgyWorker({ fullName });
  spares.set(fullName, w);
  w.initPromise.catch(() => {
    if (spares.get(fullName) === w) spares.delete(fullName);
  });
};

const takeSpare = (fullName) => {
  const w = spares.get(fullName);
  if (w) spares.delete(fullName);
  if (w && !w.dead) {
    ensureSpare(fullName); // replenish in the background
    return w;
  }
  return null;
};

const evictLiveWorkersIfNeeded = () => {
  const now = Date.now();
  for (const [lk, w] of liveWorkers) {
    if (w.dead) {
      w.kill();
      liveWorkers.delete(lk);
      continue;
    }
    // lastUsedAt is refreshed when a turn finishes, so an over-TTL worker
    // can still be actively serving a long turn. Keep it until that turn
    // settles; the next sweep can then apply the refreshed idle timestamp.
    if (w.turnWaiter !== null) continue;
    if (now - w.lastUsedAt > SESSION_IDLE_TTL_MS) {
      w.kill();
      liveWorkers.delete(lk);
    }
  }
  const idle = [...liveWorkers.entries()]
    .filter(([, w]) => w.turnWaiter === null)
    .sort((a, b) => a[1].lastUsedAt - b[1].lastUsedAt);
  while (liveWorkers.size >= MAX_LIVE_WORKERS && idle.length > 0) {
    const [lk, w] = idle.shift();
    w.kill();
    liveWorkers.delete(lk);
  }
};
setInterval(evictLiveWorkersIfNeeded, 60000).unref();

// Acquire a worker for one request. Synchronous: spawning is cheap to
// start; runTurn awaits the worker's init. `resumed` is true when the
// worker was spawned to resume `conversationId` or already holds the
// session's conversation (so the caller sends the delta prompt and may
// apply the fresh-conversation retry on a no-output failure).
const acquireWorker = ({ fullName, key, conversationId }) => {
  if (key) {
    const lk = `${fullName}|${key}`;
    const live = liveWorkers.get(lk);
    if (live && !live.dead) return { worker: live, resumed: true, lk };
    if (live) liveWorkers.delete(lk);
    evictLiveWorkersIfNeeded();
    if (conversationId) {
      const w = new AgyWorker({ fullName, conversationId });
      liveWorkers.set(lk, w);
      return { worker: w, resumed: true, lk };
    }
    const w = takeSpare(fullName) || new AgyWorker({ fullName });
    liveWorkers.set(lk, w);
    return { worker: w, resumed: false, lk };
  }
  const w = takeSpare(fullName) || new AgyWorker({ fullName });
  return { worker: w, resumed: false, lk: null };
};

// Drop a worker after a failed turn / a finished stateless turn: it is
// killed, unbound, and (for stateless use) a fresh spare is warmed.
const retireWorker = ({ worker, lk }) => {
  if (lk && liveWorkers.get(lk) === worker) liveWorkers.delete(lk);
  worker.kill();
  if (!lk) ensureSpare(worker.fullName);
};

const killAllWorkers = () => {
  for (const w of liveWorkers.values()) w.kill();
  for (const w of spares.values()) w.kill();
  liveWorkers.clear();
  spares.clear();
};
// Registered before publishEndpoint's handlers (added at listen time)
// so workers die before the process exits on SIGTERM/SIGINT.
process.on('SIGTERM', killAllWorkers);
process.on('SIGINT', killAllWorkers);
process.on('exit', killAllWorkers);

// Per-session-key serialization: turns on one conversation must not
// interleave (stateless requests use distinct workers and skip this).
const chains = new Map();
const enqueue = (key, job) => {
  const prev = chains.get(key) || Promise.resolve();
  const run = prev.then(job, job);
  const stored = run.catch(() => {});
  chains.set(key, stored);
  stored.then(() => { if (chains.get(key) === stored) chains.delete(key); });
  return run;
};

const warmStatus = () => ({
  spares: Object.fromEntries([...spares.entries()].map(([m, w]) => [m, w.pid])),
  sparePids: [...spares.values()].filter(w => !w.dead).map(w => w.pid),
  sessionPids: [...liveWorkers.values()].filter(w => !w.dead).map(w => w.pid),
  liveSessions: liveWorkers.size
});

const server = http.createServer((req, res) => {
  const url = new URL(req.url, `http://${req.headers.host || HOST}`);
  const pathname = url.pathname.replace(/\/+$/, '') || '/';

  // Health & Root
  if (req.method === 'GET' && (pathname === '/health' || pathname === '/')) {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({
      status: 'ok',
      service: 'antigravity-agy-proxy',
      port: PORT,
      binary: AGY_BIN,
      engine: 'agy-stream-json-resident',
      defaultModel: 'gemini-3.8-flash-medium',
      activeSessions: sessionStore.size,
      models: modelsStatus(),
      warm: warmStatus()
    }, null, 2));
    return;
  }

  // Models catalog (live-pulled from `agy models`, see above)
  if (req.method === 'GET' && (pathname === '/v1/models' || pathname === '/models')) {
    (async () => {
      await ensureInitialModels();
      maybeRefreshModels();
      const now = Math.floor(Date.now() / 1000);
      const data = [];

      for (const m of modelCatalog) {
        data.push({
          id: m.id,
          object: 'model',
          created: now,
          owned_by: 'antigravity-agy'
        });
        for (const eff of m.efforts) {
          data.push({
            id: `${m.id}-${eff}`,
            object: 'model',
            created: now,
            owned_by: 'antigravity-agy'
          });
        }
      }

      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ object: 'list', data }, null, 2));
    })();
    return;
  }

  // Chat completions
  if (req.method === 'POST' && (pathname === '/v1/chat/completions' || pathname === '/chat/completions')) {
    let bodyText = '';
    req.on('data', chunk => {
      bodyText += chunk;
      if (bodyText.length > 20 * 1024 * 1024) {
        res.writeHead(413, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: 'Request payload exceeds 20MB limit' }));
        req.destroy();
      }
    });

    req.on('end', async () => {
      let body;
      try {
        body = JSON.parse(bodyText);
      } catch (err) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: 'Invalid JSON request body' }));
        return;
      }

      if (typeof body !== 'object' || body === null || Array.isArray(body)) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: 'Invalid JSON request body: expected an object' }));
        return;
      }

      const stream = Boolean(body.stream);
      const requestedEffort = body.reasoning_effort || body.effort || (body.options && (body.options.reasoning_effort || body.options.effort));
      await ensureInitialModels();
      maybeRefreshModels();
      const resolved = resolveModelAndEffort(body.model, requestedEffort);
      if (!resolved) {
        // Judged against the current live / last-known-good list; a
        // miss also triggers an async re-pull so a model agy gained
        // since the last pull is picked up without waiting for TTL.
        refreshModels();
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: `Unknown model: ${body.model}` }));
        return;
      }
      const { fullName } = resolved;

      const sessionInfo = getSessionKey(req, body);
      const isResuming = Boolean(sessionInfo.conversationId);
      const prompt = isResuming
        ? extractLatestUserPrompt(body.messages)
        : formatPrompt(body.messages);

      if (!prompt.trim()) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: 'Messages list produced empty prompt' }));
        return;
      }

      const reqId = `chatcmpl-${crypto.randomUUID()}`;
      const created = Math.floor(Date.now() / 1000);

      let currentWorker = null;
      let activeConversationId = sessionInfo.conversationId || null;

      res.on('close', () => {
        if (!res.writableEnded && currentWorker) {
          const lk = sessionInfo.key ? `${fullName}|${sessionInfo.key}` : null;
          retireWorker({ worker: currentWorker, lk });
        }
      });

      const recordConversationId = (id) => {
        if (!id) return;
        activeConversationId = id;
        if (sessionInfo.key) {
          sessionStore.set(sessionInfo.key, { conversationId: id, updatedAt: Date.now() });
          pruneSessionStore();
        }
      };

      // Streaming head is written lazily (on the first delta, or when
      // the turn settles) so a no-output resume failure can still be
      // retried transparently before anything reaches the client.
      let headed = false;
      const ensureStreamHead = () => {
        if (!stream || headed) return;
        headed = true;
        res.writeHead(200, {
          'Content-Type': 'text/event-stream',
          'Cache-Control': 'no-cache',
          'Connection': 'keep-alive',
          'X-Accel-Buffering': 'no'
        });
      };

      const runJob = async () => {
        const attempt = async (targetConvId, promptText, allowRetry) => {
          const { worker, resumed, lk } = acquireWorker({
            fullName, key: sessionInfo.key, conversationId: targetConvId
          });
          currentWorker = worker;
          try { res.setHeader('X-Agy-Worker-Pid', String(worker.pid)); } catch {}
          if (worker.conversationId) activeConversationId = worker.conversationId;

          let streamedText = '';
          const onDelta = stream ? (delta) => {
            streamedText += delta;
            ensureStreamHead();
            const chunk = {
              id: reqId,
              object: 'chat.completion.chunk',
              created,
              model: fullName,
              conversation_id: activeConversationId,
              choices: [
                {
                  index: 0,
                  delta: { content: delta },
                  finish_reason: null
                }
              ]
            };
            res.write(`data: ${JSON.stringify(chunk)}\n\n`);
          } : null;

          let result;
          const turnStart = Date.now();
          try {
            result = await worker.runTurn(promptText, onDelta);
          } catch (err) {
            const durationMs = Date.now() - turnStart;
            console.log(`[agy-proxy] turn reqId=${reqId} model=${fullName} workerPid=${worker.pid} durationMs=${durationMs} status=error`);
            try { if (!res.headersSent) res.setHeader('X-Agy-Turn-Ms', String(durationMs)); } catch {}
            retireWorker({ worker, lk });
            if (currentWorker === worker) currentWorker = null;
            // Fallback retry if resuming failed without output (#678's
            // retry, preserved): start over as a fresh conversation
            // carrying the full flattened prompt.
            if (allowRetry && resumed && !err.hasOutput) {
              console.log(`Resuming conversation ${targetConvId} failed (${err.detail}), retrying as fresh conversation...`);
              return attempt(null, formatPrompt(body.messages), false);
            }
            // Failure is judged by the turn's outcome, never by whether
            // partial output was produced (#678): partial output must
            // not mask a failure as a successful 'stop'.
            if (stream) {
              ensureStreamHead();
              const errorBody = { error: { message: `agy execution failed: ${err.detail}` } };
              // The 200 + text/event-stream headers may already be on
              // the wire, so frame the error as an SSE event and close
              // the stream with [DONE] (mirrors muse-proxy #670).
              res.write(`data: ${JSON.stringify(errorBody)}\n\n`);
              res.write('data: [DONE]\n\n');
              res.end();
            } else if (err.isExitError) {
              res.writeHead(502, { 'Content-Type': 'application/json' });
              res.end(JSON.stringify({
                error: 'agy process exited with non-zero code',
                code: err.code,
                ...(err.signal ? { signal: err.signal } : {}),
                stderr: (err.stderr || '').trim()
              }));
            } else {
              res.writeHead(502, { 'Content-Type': 'application/json' });
              res.end(JSON.stringify({
                error: `agy execution failed: ${err.detail}`,
                stderr: (err.stderr || '').trim()
              }));
            }
            return;
          }

          // Success: commit the session mapping, keep session workers
          // resident for their next turn, retire stateless workers (a
          // replacement spare is already warming).
          const durationMs = Date.now() - turnStart;
          console.log(`[agy-proxy] turn reqId=${reqId} model=${fullName} workerPid=${worker.pid} durationMs=${durationMs} status=ok`);
          try { if (!res.headersSent) res.setHeader('X-Agy-Turn-Ms', String(durationMs)); } catch {}
          recordConversationId(result.conversationId);
          if (!lk) retireWorker({ worker, lk });
          if (currentWorker === worker) currentWorker = null;

          if (stream) {
            // Emit any response tail the result carried beyond the
            // deltas already streamed (short replies can arrive whole
            // in the result with no ACTIVE delta for the tail).
            if (typeof result.text === 'string' && result.text.startsWith(streamedText) && result.text.length > streamedText.length) {
              const suffix = result.text.slice(streamedText.length);
              ensureStreamHead();
              const chunk = {
                id: reqId,
                object: 'chat.completion.chunk',
                created,
                model: fullName,
                conversation_id: activeConversationId,
                choices: [{ index: 0, delta: { content: suffix }, finish_reason: null }]
              };
              res.write(`data: ${JSON.stringify(chunk)}\n\n`);
            }
            ensureStreamHead();
            const finishChunk = {
              id: reqId,
              object: 'chat.completion.chunk',
              created,
              model: fullName,
              conversation_id: activeConversationId,
              choices: [
                {
                  index: 0,
                  delta: {},
                  finish_reason: 'stop'
                }
              ],
              usage: {
                prompt_tokens: result.usage?.input_tokens || 0,
                completion_tokens: result.usage?.output_tokens || 0,
                total_tokens: result.usage?.total_tokens || 0,
                ...(result.usage?.cache_read_tokens > 0 ? { prompt_tokens_details: { cached_tokens: result.usage.cache_read_tokens } } : {})
              }
            };
            res.write(`data: ${JSON.stringify(finishChunk)}\n\n`);
            res.write('data: [DONE]\n\n');
            res.end();
          } else {
            res.writeHead(200, {
              'Content-Type': 'application/json',
              ...(activeConversationId ? {
                'X-Conversation-Id': activeConversationId,
                'X-Session-Id': activeConversationId
              } : {})
            });
            res.end(JSON.stringify({
              id: reqId,
              object: 'chat.completion',
              created,
              model: fullName,
              conversation_id: activeConversationId,
              choices: [
                {
                  index: 0,
                  message: {
                    role: 'assistant',
                    content: result.text
                  },
                  finish_reason: 'stop'
                }
              ],
              usage: {
                prompt_tokens: result.usage?.input_tokens || 0,
                completion_tokens: result.usage?.output_tokens || 0,
                total_tokens: result.usage?.total_tokens || 0,
                ...(result.usage?.cache_read_tokens > 0 ? { prompt_tokens_details: { cached_tokens: result.usage.cache_read_tokens } } : {})
              }
            }, null, 2));
          }
        };

        await attempt(sessionInfo.conversationId, prompt, Boolean(sessionInfo.conversationId));
      };

      try {
        if (sessionInfo.key) {
          await enqueue(`${fullName}|${sessionInfo.key}`, runJob);
        } else {
          await runJob();
        }
      } catch (err) {
        if (!res.writableEnded && !res.headersSent) {
          res.writeHead(500, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ error: err.message }));
        } else if (!res.writableEnded) {
          res.end();
        }
      }
    });
    return;
  }

  res.writeHead(404, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ error: 'Endpoint not found', path: pathname }));
});

server.listen(PORT, HOST, () => {
  publishEndpoint();
  for (const m of PREWARM_MODELS) ensureSpare(m);
  console.log(`Antigravity agy Proxy listening on http://${HOST}:${PORT}`);
  console.log(`Engine: resident agy stream-json workers (spares ${SPARES_ENABLED ? 'on' : 'off'}, prewarm: ${PREWARM_MODELS.join(', ') || 'none'})`);
  console.log(`Endpoints:`);
  console.log(`  - GET  http://${HOST}:${PORT}/health`);
  console.log(`  - GET  http://${HOST}:${PORT}/v1/models`);
  console.log(`  - POST http://${HOST}:${PORT}/v1/chat/completions`);
});
