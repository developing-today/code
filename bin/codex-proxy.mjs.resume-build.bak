#!/usr/bin/env node
import http from 'node:http';
import { spawn, execSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { parseProxyPort } from './proxy-port.mjs';

const PORT = parseProxyPort('CODEX_PROXY_PORT', 18744);
const HOST = process.env.CODEX_PROXY_HOST || '127.0.0.1';

// Publish the bound endpoint so other tools can find this proxy without
// knowing its port in advance: $XDG_STATE_HOME/<name>/endpoint.json
// (default ~/.local/state/<name>/endpoint.json), written on listen,
// removed on exit.
const publishEndpoint = () => {
  try {
    const stateHome =
      process.env.XDG_STATE_HOME || path.join(os.homedir(), '.local', 'state');
    const dir = path.join(stateHome, 'codex-proxy');
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

// Synchronize freshly refreshed ChatGPT OAuth tokens from Codex to OpenCode SQLite DB
const syncCodexTokensToOpenCode = () => {
  try {
    const codexAuthPath = `${process.env.HOME}/.codex/auth.json`;
    const dbPath = `${process.env.HOME}/.local/share/opencode/opencode.db`;
    if (!fs.existsSync(codexAuthPath) || !fs.existsSync(dbPath)) return;

    const codexAuth = JSON.parse(fs.readFileSync(codexAuthPath, 'utf-8'));
    const tokens = codexAuth.tokens;
    if (!tokens || !tokens.access_token) return;

    const script = `import sqlite3, json, time
conn = sqlite3.connect('${dbPath}')
c = conn.cursor()
c.execute('SELECT id, value FROM credential WHERE integration_id="openai"')
for row in c.fetchall():
    val = json.loads(row[1])
    if val.get('type') == 'oauth':
        val['access'] = ${JSON.stringify(tokens.access_token)}
        if ${JSON.stringify(tokens.refresh_token || '')}:
            val['refresh'] = ${JSON.stringify(tokens.refresh_token || '')}
        if ${JSON.stringify(tokens.account_id || '')}:
            val['metadata'] = val.get('metadata') or {}
            val['metadata']['accountID'] = ${JSON.stringify(tokens.account_id || '')}
        val['expires'] = int(time.time() * 1000) + 30 * 86400 * 1000
        c.execute('UPDATE credential SET value = ?, time_updated = ? WHERE id = ?', (json.dumps(val), int(time.time() * 1000), row[0]))
conn.commit()
`;
    execSync(`python3 -c ${JSON.stringify(script)}`, { stdio: 'ignore' });
  } catch (err) {
    // Non-fatal background sync
  }
};

const MODELS = [
  { id: 'gpt-6.1-sol', name: 'GPT-6.1 Sol (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'], defaultEffort: 'medium' },
  { id: 'gpt-6-astra', name: 'GPT-6 Astra (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'], defaultEffort: 'medium' },
  { id: 'gpt-6-sol', name: 'GPT-6 Sol (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'], defaultEffort: 'medium' },
  { id: 'gpt-6-luna', name: 'GPT-6 Luna (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'low' },
  { id: 'gpt-5.6-sol', name: 'GPT-5.6 Sol (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'], defaultEffort: 'medium' },
  { id: 'gpt-5.6-terra', name: 'GPT-5.6 Terra (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'], defaultEffort: 'medium' },
  { id: 'gpt-5.6-luna', name: 'GPT-5.6 Luna (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'low' },
  { id: 'gpt-5.5', name: 'GPT-5.5 (codex)', efforts: ['low', 'medium', 'high', 'xhigh'], defaultEffort: 'low' },
  { id: 'gpt-reserve', name: 'GPT Reserve (codex)', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'low' },
  { id: 'codex-auto-review', name: 'Codex Auto Review', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'medium' }
];

const findCodexBinary = () => {
  const candidates = [
    '/run/current-system/sw/bin/codex',
    `${process.env.HOME}/.local/bin/codex`,
    'codex'
  ];
  for (const c of candidates) {
    if (c === 'codex' || fs.existsSync(c)) {
      return c;
    }
  }
  return 'codex';
};

const CODEX_BIN = findCodexBinary();

const resolveModelAndEffort = (requestedModel, requestedEffort) => {
  let rawModel = (requestedModel || 'gpt-6.1-sol').trim();
  // Strip any provider prefix before '/' (e.g. 'openai-codex/', 'codex/', 'openai/', 'chatgpt/')
  let modelStr = rawModel.replace(/^.*\//, '').toLowerCase().trim();
  let effort = requestedEffort ? requestedEffort.toLowerCase().trim() : null;

  // Check if model string ends with effort suffix
  const possibleSuffixes = ['-ultra', '-xhigh', '-high', '-medium', '-low', '-max'];
  for (const suffix of possibleSuffixes) {
    if (modelStr.endsWith(suffix)) {
      effort = suffix.slice(1);
      modelStr = modelStr.slice(0, -suffix.length);
      break;
    }
  }

  // Find base model config
  const modelConfig = MODELS.find(m => m.id === modelStr || m.id.replace(/-/g, '') === modelStr.replace(/-/g, ''))
    || MODELS[0]; // default to gpt-6.1-sol

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

// ---------------------------------------------------------------------------
// Warm sessions (#480): conversation-keyed resumable `codex exec` sessions.
//
// Before this, every request spawned a fresh `codex exec --ephemeral` and
// re-flattened the whole conversation, so the model re-read the entire
// history on every turn. Now the first turn of a conversation runs WITHOUT
// --ephemeral (an ephemeral session leaves no rollout on disk and can
// never be resumed), captures the session id from the CLI's JSON event
// stream, and later turns of the same conversation run
// `codex exec resume <id>` with only the new tail.
//
// Conversations are keyed by sha256(resolved model/effort + system prompt
// + first user message): the proxy is stateless per request and that is
// all it can stably recognise a conversation by. The map is persisted next
// to endpoint.json (sessions.json in the same state dir) so a proxy
// restart does not cold-start every conversation, LRU-bounded to 64
// entries with a 2h TTL. Any resume failure falls back to a full-history
// fresh run and re-keys the entry — a request never fails because resume
// failed.
// ---------------------------------------------------------------------------
const SESSION_TTL_MS = 2 * 60 * 60 * 1000;
const SESSION_MAX_ENTRIES = 64;

const sessionStateDir = () =>
  path.join(
    process.env.XDG_STATE_HOME || path.join(os.homedir(), '.local', 'state'),
    'codex-proxy'
  );

let sessionMap = null; // Map<key, {sessionId, fedCount, fedHashes, lastUsed}>

const loadSessions = () => {
  if (sessionMap) return sessionMap;
  sessionMap = new Map();
  try {
    const raw = JSON.parse(
      fs.readFileSync(path.join(sessionStateDir(), 'sessions.json'), 'utf-8')
    );
    const now = Date.now();
    for (const [key, entry] of Object.entries(raw)) {
      if (
        entry &&
        typeof entry.sessionId === 'string' &&
        Array.isArray(entry.fedHashes) &&
        now - (entry.lastUsed || 0) <= SESSION_TTL_MS
      ) {
        sessionMap.set(key, entry);
      }
    }
  } catch {
    // No state yet — cold start.
  }
  return sessionMap;
};

const saveSessions = () => {
  try {
    const entries = [...loadSessions().entries()]
      .sort((a, b) => (b[1].lastUsed || 0) - (a[1].lastUsed || 0))
      .slice(0, SESSION_MAX_ENTRIES);
    sessionMap = new Map(entries);
    const dir = sessionStateDir();
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, 'sessions.json');
    fs.writeFileSync(`${file}.tmp`, JSON.stringify(Object.fromEntries(entries)) + '\n');
    fs.renameSync(`${file}.tmp`, file);
  } catch {
    // Persistence is best-effort; the in-memory map still works.
  }
};

const messageText = (m) => {
  if (typeof m.content === 'string') return m.content;
  if (Array.isArray(m.content)) {
    return m.content.map(c => c.text || JSON.stringify(c)).join('\n');
  }
  return m.content ? JSON.stringify(m.content) : '';
};

const messageHash = (m) =>
  crypto
    .createHash('sha256')
    .update(
      `${m.role}\n${messageText(m)}\n${m.tool_calls ? JSON.stringify(m.tool_calls) : ''}\n${m.tool_call_id || ''}`
    )
    .digest('hex');

const conversationKey = (messages, fullName) => {
  const systemText = messages
    .filter(m => m.role === 'system')
    .map(messageText)
    .join('\n\n');
  const firstUser = messages.find(m => m.role === 'user');
  return crypto
    .createHash('sha256')
    .update(`${fullName}\n${systemText}\n${firstUser ? messageText(firstUser) : ''}`)
    .digest('hex');
};

// Decide whether this request can resume a warm session. Returns
// { key, entry, tailMessages } or null for a full-history fresh run.
const planResume = (messages, fullName) => {
  if (!Array.isArray(messages) || messages.length === 0) return null;
  const key = conversationKey(messages, fullName);
  const entry = loadSessions().get(key);
  if (!entry) return null;
  if (Date.now() - (entry.lastUsed || 0) > SESSION_TTL_MS) {
    loadSessions().delete(key);
    saveSessions();
    return null;
  }
  // The request must extend exactly what the session was fed: every
  // previously fed message still present, unchanged, at the same index.
  // A changed system prompt (or any edited/truncated history) fails here
  // and falls back to a full run.
  if (messages.length <= entry.fedCount || entry.fedHashes.length !== entry.fedCount) {
    return null;
  }
  for (let i = 0; i < entry.fedCount; i++) {
    if (messageHash(messages[i]) !== entry.fedHashes[i]) return null;
  }
  // The session already holds its own assistant replies, so the tail fed
  // to `exec resume` is only the new non-assistant turns.
  const tailMessages = messages.slice(entry.fedCount).filter(m => m.role !== 'assistant');
  if (!tailMessages.some(m => m.role === 'user' || m.role === 'tool')) return null;
  return { key, entry, tailMessages };
};

const recordSession = (key, sessionId, messages) => {
  if (!key || !sessionId) return;
  loadSessions().set(key, {
    sessionId,
    fedCount: messages.length,
    fedHashes: messages.map(messageHash),
    lastUsed: Date.now()
  });
  saveSessions();
};

const forgetSession = (key) => {
  if (key && loadSessions().delete(key)) saveSessions();
};

// Run one `codex exec` child to completion. Resolves (never rejects) to
// { ok, content, promptTokens, completionTokens, sessionId, detail }.
// `emit(text)` is called live for each completed agent message when
// given; pass null to buffer instead (resume attempts buffer, so a failed
// resume can fall back to a full run without duplicating streamed text).
const runCodexOnce = ({ args, prompt, emit, onSpawn }) =>
  new Promise((resolve) => {
    let settled = false;
    const done = (result) => {
      if (!settled) {
        settled = true;
        resolve(result);
      }
    };

    let child;
    try {
      child = spawn(CODEX_BIN, args, {
        cwd: '/tmp',
        env: {
          ...process.env,
          PATH: `/run/current-system/sw/bin:${process.env.PATH || ''}`
        },
        stdio: ['pipe', 'pipe', 'pipe']
      });
    } catch (err) {
      done({ ok: false, content: '', sessionId: null, detail: err.message });
      return;
    }
    if (onSpawn) onSpawn(child);

    console.log(`[codex-proxy] Spawning: ${CODEX_BIN} ${args.join(' ')}`);

    // Write prompt to stdin and close stdin
    child.stdin.write(prompt);
    child.stdin.end();

    let lineBuffer = '';
    let fullContent = '';
    let promptTokens = Math.max(1, Math.ceil(prompt.length / 4));
    let completionTokens = 0;
    let stderrTail = '';
    let spawnError = null;
    // In-band failure state: `codex exec --json` can report a fatal
    // turn failure as a turn.failed event on a run that still exits 0
    // (#680). null = no turn.failed seen; otherwise its error message
    // ('' when the event carried none).
    let turnFailedMessage = null;
    let sessionId = null;

    const processJsonLine = (line) => {
      const trimmed = line.trim();
      if (!trimmed || !trimmed.startsWith('{')) return;

      try {
        const event = JSON.parse(trimmed);

        // Session id for warm resume (#480): `codex exec --json` opens
        // with {"type":"thread.started","thread_id":"<uuid>"} (verified
        // against codex-cli 0.160.0). Accept the session_id spellings
        // other CLI versions have used too; first one wins.
        if (!sessionId) {
          const candidate =
            event.thread_id ||
            event.session_id ||
            (event.thread && event.thread.id) ||
            (event.session && event.session.id);
          if (typeof candidate === 'string' && candidate) sessionId = candidate;
        }

        // Agent message completed
        if (
          event.type === 'item.completed' &&
          event.item &&
          event.item.type === 'agent_message' &&
          typeof event.item.text === 'string'
        ) {
          fullContent += event.item.text;
          if (emit) emit(event.item.text);
        }

        // Turn failed in-band: a fatal error during the turn. The
        // exit code cannot be trusted to reflect it (#680).
        if (event.type === 'turn.failed') {
          turnFailedMessage =
            typeof event.error === 'string'
              ? event.error
              : event.error && typeof event.error.message === 'string'
                ? event.error.message
                : '';
        }

        // Turn completed with usage metrics
        if (event.type === 'turn.completed' && event.usage) {
          if (typeof event.usage.input_tokens === 'number') {
            promptTokens = event.usage.input_tokens;
          }
          if (typeof event.usage.output_tokens === 'number') {
            completionTokens = event.usage.output_tokens;
          }
        }
      } catch (err) {
        // Non-JSON line or parser error ignored
      }
    };

    child.stdout.on('data', (chunk) => {
      lineBuffer += chunk.toString('utf-8');
      const lines = lineBuffer.split('\n');
      lineBuffer = lines.pop(); // remainder
      for (const line of lines) {
        processJsonLine(line);
      }
    });

    child.stderr.on('data', (chunk) => {
      const msg = chunk.toString('utf-8');
      stderrTail = (stderrTail + msg).slice(-2000);
      if (msg.trim().length > 0) {
        console.warn(`[codex-proxy stderr] ${msg.trim()}`);
      }
    });

    child.on('close', (code, signal) => {
      if (lineBuffer.trim().length > 0) {
        processJsonLine(lineBuffer);
        lineBuffer = '';
      }

      if (completionTokens === 0 && fullContent.length > 0) {
        completionTokens = Math.max(1, Math.ceil(fullContent.length / 4));
      }

      // Failure is judged by the run's authoritative signals: a
      // turn.failed event (#680), or failing that the child's exit —
      // a non-zero code or a killing signal means the run failed,
      // however much content was already produced (#676). Partial
      // content must not mask a failure.
      if (code !== 0 || turnFailedMessage !== null) {
        const stderrLine = stderrTail.trim().split('\n').pop();
        const fallback = spawnError
          ? spawnError.message
          : signal
            ? `codex exec terminated by signal ${signal}`
            : `codex exec exited with code ${code}`;
        const detail = (turnFailedMessage || stderrLine || fallback).trim();
        done({
          ok: false,
          content: fullContent,
          promptTokens,
          completionTokens,
          sessionId,
          detail
        });
        return;
      }

      done({
        ok: true,
        content: fullContent,
        promptTokens,
        completionTokens,
        sessionId,
        detail: null
      });
    });

    child.on('error', (err) => {
      console.error(`[codex-proxy child error] ${err.message}`);
      spawnError = err;
    });
  });

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
      service: 'openai-codex-proxy',
      port: PORT,
      binary: CODEX_BIN,
      defaultModel: 'gpt-5.6-luna-low'
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
        owned_by: 'openai-codex'
      });
      for (const eff of m.efforts) {
        data.push({
          id: `${m.id}-${eff}`,
          object: 'model',
          created: now,
          owned_by: 'openai-codex'
        });
      }
    }

    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ object: 'list', data }, null, 2));
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

    req.on('end', () => {
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

      const chatId = `chatcmpl-codex-${crypto.randomUUID()}`;
      const created = Math.floor(Date.now() / 1000);

      // Spawn arguments for codex exec. Fresh runs deliberately omit
      // --ephemeral: an ephemeral session leaves no rollout on disk, so
      // it could never be resumed by a later turn (#480).
      const commonArgs = [
        '--json',
        '--skip-git-repo-check',
        '--dangerously-bypass-approvals-and-sandbox',
        '--disable', 'shell_tool',
        '-m', baseModel,
        '-c', `model_reasoning_effort="${effort}"`,
        '-c', 'model_context_window=1000000',
        '-c', 'model_auto_compact_token_limit=950000'
      ];
      const freshArgs = ['exec', ...commonArgs, '--color', 'never', '-'];
      // `exec resume` has no --color flag in codex-cli 0.160.0 (passing
      // it is a usage error, exit 2 — verified on amd); the rest of the
      // option set is shared with `exec`.
      const resumeArgs = (sessionId) => ['exec', 'resume', ...commonArgs, sessionId, '-'];

      // Refresh OpenCode tokens in background
      syncCodexTokensToOpenCode();

      if (stream) {
        res.writeHead(200, {
          'Content-Type': 'text/event-stream; charset=utf-8',
          'Cache-Control': 'no-cache, no-transform',
          'Connection': 'keep-alive',
          'X-Accel-Buffering': 'no'
        });
      }

      const writeChunk = (delta) => {
        if (res.writableEnded) return;
        const chunk = {
          id: chatId,
          object: 'chat.completion.chunk',
          created,
          model: fullName,
          choices: [{ index: 0, delta, finish_reason: null }]
        };
        res.write(`data: ${JSON.stringify(chunk)}\n\n`);
      };

      const sendSuccess = (result, replay) => {
        if (res.writableEnded) return;
        const usage = {
          prompt_tokens: result.promptTokens,
          completion_tokens: result.completionTokens,
          total_tokens: result.promptTokens + result.completionTokens
        };
        if (stream) {
          // Buffered (resumed) runs replay their content as one chunk;
          // live runs already streamed each item as it completed.
          if (replay && result.content) writeChunk({ content: result.content });
          const endChunk = {
            id: chatId,
            object: 'chat.completion.chunk',
            created,
            model: fullName,
            choices: [{ index: 0, delta: {}, finish_reason: 'stop' }],
            usage
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
              message: { role: 'assistant', content: result.content },
              finish_reason: 'stop'
            }],
            usage
          }, null, 2));
        }
      };

      const sendFailure = (detail) => {
        if (res.writableEnded) return;
        const errorBody = { error: { message: `Codex execution failed: ${detail}` } };
        if (stream) {
          // The 200 + text/event-stream headers are already on the
          // wire, so frame the error as an SSE event and close the
          // stream with [DONE] (mirrors muse-proxy #670).
          res.write(`data: ${JSON.stringify(errorBody)}\n\n`);
          res.write('data: [DONE]\n\n');
          res.end();
          return;
        }
        if (!res.headersSent) {
          res.writeHead(500, { 'Content-Type': 'application/json' });
        }
        res.end(JSON.stringify(errorBody));
      };

      (async () => {
        let currentChild = null;
        res.on('close', () => {
          if (!res.writableEnded && currentChild) {
            try { currentChild.kill('SIGTERM'); } catch (_) {}
          }
        });
        const onSpawn = (c) => { currentChild = c; };

        // Warm path (#480): resume the conversation's codex session
        // with only the new tail. Buffered, so a failed resume can fall
        // back to a full-history fresh run without duplicating text.
        const plan = planResume(body.messages, fullName);
        if (plan) {
          const tailPrompt = formatPrompt(plan.tailMessages);
          const attempt = await runCodexOnce({
            args: resumeArgs(plan.entry.sessionId),
            prompt: tailPrompt,
            emit: null,
            onSpawn
          });
          if (attempt.ok) {
            recordSession(plan.key, attempt.sessionId || plan.entry.sessionId, body.messages);
            console.log(
              `[codex-proxy] warm resume: key=${plan.key.slice(0, 8)} session=${plan.entry.sessionId} fed=${plan.entry.fedCount}->${body.messages.length}`
            );
            sendSuccess(attempt, true);
            return;
          }
          console.warn(
            `[codex-proxy] warm resume failed (${attempt.detail}); falling back to full-history run`
          );
          forgetSession(plan.key);
        }

        // Cold path: full flattened history, exactly as before — except
        // the session is now persisted and recorded for warm resume.
        const key = conversationKey(body.messages, fullName);
        const result = await runCodexOnce({
          args: freshArgs,
          prompt,
          emit: stream ? (text) => writeChunk({ content: text }) : null,
          onSpawn
        });
        if (!result.ok) {
          sendFailure(result.detail);
          return;
        }
        recordSession(key, result.sessionId, body.messages);
        sendSuccess(result, false);
      })().catch((err) => {
        console.error(`[codex-proxy] request handler error: ${err && err.message}`);
        sendFailure(err && err.message ? err.message : 'internal error');
      });
    });
    return;
  }

  // Fallthrough 404
  res.writeHead(404, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ error: { message: `Route not found: ${req.method} ${pathname}` } }));
});

server.listen(PORT, HOST, () => {
  publishEndpoint();
  console.log(`[codex-proxy] OpenAI-compatible bridge listening on http://${HOST}:${PORT}`);
  console.log(`[codex-proxy] Resolved codex binary: ${CODEX_BIN}`);
  syncCodexTokensToOpenCode();
  setInterval(syncCodexTokensToOpenCode, 5 * 60 * 1000);
});
