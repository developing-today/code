#!/usr/bin/env node
import http from 'node:http';
import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { parseProxyPort } from './proxy-port.mjs';

const PORT = parseProxyPort('AGY_PROXY_PORT', 18743);
const HOST = process.env.AGY_PROXY_HOST || '127.0.0.1';

const stateDir = () => {
  const stateHome =
    process.env.XDG_STATE_HOME || path.join(os.homedir(), '.local', 'state');
  return path.join(stateHome, 'agy-proxy');
};

// Publish the bound endpoint so other tools can find this proxy without
// knowing its port in advance: $XDG_STATE_HOME/<name>/endpoint.json
// (default ~/.local/state/<name>/endpoint.json), written on listen,
// removed on exit.
const publishEndpoint = () => {
  try {
    const dir = stateDir();
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

const MODELS = [
  { id: 'gemini-3.8-flash', name: 'Gemini 3.8 Flash (agy)', efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' },
  { id: 'gemini-3.7-flash', name: 'Gemini 3.7 Flash (agy)', efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' },
  { id: 'gemini-3.6-flash', name: 'Gemini 3.6 Flash (agy)', efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' },
  { id: 'gemini-3.1-pro', name: 'Gemini 3.1 Pro (agy)', efforts: ['low', 'high'], defaultEffort: 'high' },
  { id: 'claude-sonnet-5-5', name: 'Claude Sonnet 5.5 (agy)', efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' },
  { id: 'claude-opus-5-5', name: 'Claude Opus 5.5 (agy)', efforts: ['low', 'medium', 'high'], defaultEffort: 'medium' },
  { id: 'gpt-oss-120b', name: 'GPT-OSS 120B (agy)', efforts: ['medium'], defaultEffort: 'medium' }
];

const findAgyBinary = () => {
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

  // Find base model config
  const modelConfig = MODELS.find(m => m.id === modelStr || m.id.replace(/-/g, '') === modelStr.replace(/-/g, ''))
    || MODELS[0];

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

  // Automatic multi-turn tracking is handled by the warm-session store
  // below (keyed on system + first user message + model, persisted).
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
// Warm sessions (#480): reuse one `agy` conversation per logical chat
// conversation instead of re-flattening the full history into a fresh CLI
// process on every request.
//
// How the conversation id is learned (probed against agy 1.4.1):
//   --output-format json       -> top-level {"conversation_id": "<id>", ...}
//   --output-format stream-json-> {"event":"result","result":
//                                  {"conversation_id": "<id>", ...}}
// and a run is resumed with `--conversation <id> -p <new turn only>`.
// A bogus id does NOT fail the run: the CLI prints
// `warning: conversation "<id>" not found` on stderr and silently starts
// a fresh conversation, so that warning is treated as a resume failure.
//
// Key: sha256(system prompt + first user message + resolved model).
// Entry: { conversationId, fedCount, fedHashes, lastResponseHash,
//          lastUsed } — fedCount/fedHashes record exactly which request
// messages the agy conversation has already consumed, so a hit is only
// used when the new history is a strict continuation of what was fed.
// Persisted to warm-sessions.json next to endpoint.json; LRU 64 / 2h TTL.
// Any resume failure falls back to a full-history fresh run.
// ---------------------------------------------------------------------------
const WARM_MAX_ENTRIES = 64;
const WARM_TTL_MS = 2 * 60 * 60 * 1000;
const warmStore = new Map();

const warmFile = () => path.join(stateDir(), 'warm-sessions.json');

const persistWarmStore = () => {
  try {
    fs.mkdirSync(stateDir(), { recursive: true });
    const file = warmFile();
    fs.writeFileSync(`${file}.tmp`, JSON.stringify(Object.fromEntries(warmStore)) + '\n');
    fs.renameSync(`${file}.tmp`, file);
  } catch {}
};

const pruneWarmStore = () => {
  const now = Date.now();
  for (const [key, entry] of warmStore) {
    if (!entry || !entry.conversationId || now - (entry.lastUsed || 0) > WARM_TTL_MS) {
      warmStore.delete(key);
    }
  }
  if (warmStore.size > WARM_MAX_ENTRIES) {
    const entries = Array.from(warmStore.entries())
      .sort((a, b) => (a[1].lastUsed || 0) - (b[1].lastUsed || 0));
    for (const [key] of entries.slice(0, entries.length - WARM_MAX_ENTRIES)) {
      warmStore.delete(key);
    }
  }
};

const loadWarmStore = () => {
  try {
    const parsed = JSON.parse(fs.readFileSync(warmFile(), 'utf8'));
    for (const [key, entry] of Object.entries(parsed)) {
      if (entry && typeof entry.conversationId === 'string' && entry.conversationId) {
        warmStore.set(key, entry);
      }
    }
    pruneWarmStore();
  } catch {}
};
loadWarmStore();

const messageText = (m) => {
  if (!m) return '';
  if (typeof m.content === 'string') return m.content;
  if (Array.isArray(m.content)) {
    return m.content.map(c => c.text || JSON.stringify(c)).join('\n');
  }
  return m.content == null ? '' : JSON.stringify(m.content);
};

const hashText = (text) => crypto.createHash('sha256').update(text).digest('hex').slice(0, 32);
const hashMessage = (m) => hashText(`${m?.role || ''}\n${messageText(m)}`);

const computeWarmKey = (messages, fullName) => {
  if (!Array.isArray(messages)) return null;
  const firstUser = messages.find(m => m.role === 'user');
  if (!firstUser) return null;
  const systemText = messages
    .filter(m => m.role === 'system')
    .map(messageText)
    .join('\n\n');
  return crypto.createHash('sha256')
    .update(JSON.stringify([systemText, messageText(firstUser), fullName]))
    .digest('hex');
};

// Decide fresh vs resume for a request with no explicit session id.
// Returns { key, mode: 'fresh' } or
//         { key, mode: 'resume', entry, prompt } where prompt carries
//         ONLY the messages the agy conversation has not seen yet.
const planWarmRequest = (messages, fullName) => {
  const key = computeWarmKey(messages, fullName);
  if (!key) return { key: null, mode: 'fresh' };
  pruneWarmStore();
  const entry = warmStore.get(key);
  if (!entry || !entry.conversationId) return { key, mode: 'fresh' };
  // History must be a strict continuation of what was fed: more messages
  // than fedCount, with an identical fed prefix.
  if (!Array.isArray(messages) || messages.length <= entry.fedCount) return { key, mode: 'fresh' };
  if (!Array.isArray(entry.fedHashes) || entry.fedHashes.length < entry.fedCount) return { key, mode: 'fresh' };
  for (let i = 0; i < entry.fedCount; i++) {
    if (hashMessage(messages[i]) !== entry.fedHashes[i]) return { key, mode: 'fresh' };
  }
  let idx = entry.fedCount;
  // The assistant message the client echoes back first is the response
  // the resumed conversation itself produced — already in its history.
  // Verify it is that response; a tampered/different echo is a mismatch.
  if (messages[idx]?.role === 'assistant') {
    if (entry.lastResponseHash && hashText(messageText(messages[idx])) !== entry.lastResponseHash) {
      return { key, mode: 'fresh' };
    }
    idx++;
  }
  const remaining = messages.slice(idx);
  if (remaining.length === 0) return { key, mode: 'fresh' };
  if (!remaining.some(m => m.role === 'user')) return { key, mode: 'fresh' };
  if (remaining.some(m => m.role === 'system')) return { key, mode: 'fresh' };
  const prompt = remaining.length === 1 && remaining[0].role === 'user'
    ? messageText(remaining[0])
    : formatPrompt(remaining);
  if (!prompt.trim()) return { key, mode: 'fresh' };
  return { key, mode: 'resume', entry, prompt };
};

const commitWarm = (key, conversationId, messages, responseText) => {
  if (!key) return;
  if (!conversationId) {
    if (warmStore.delete(key)) persistWarmStore();
    return;
  }
  warmStore.set(key, {
    conversationId,
    fedCount: messages.length,
    fedHashes: messages.map(hashMessage),
    lastResponseHash: responseText ? hashText(responseText) : null,
    lastUsed: Date.now()
  });
  pruneWarmStore();
  persistWarmStore();
};

const invalidateWarm = (key) => {
  if (key && warmStore.delete(key)) persistWarmStore();
};

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
      defaultModel: 'gemini-3.8-flash-medium',
      activeSessions: sessionStore.size,
      warmSessions: warmStore.size
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

    req.on('end', () => {
      let body;
      try {
        body = JSON.parse(bodyText);
      } catch (err) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: 'Invalid JSON request body' }));
        return;
      }

      const stream = Boolean(body.stream);
      const requestedEffort = body.reasoning_effort || body.effort || (body.options && (body.options.reasoning_effort || body.options.effort));
      const { baseModel, effort, fullName } = resolveModelAndEffort(body.model, requestedEffort);

      const sessionInfo = getSessionKey(req, body);
      // Warm keying applies only when the caller did not pin a session:
      // an explicit id or a cached `user:` session keeps the legacy
      // latest-turn-only resume behaviour.
      const warmPlan = (!sessionInfo.conversationId && !sessionInfo.explicit)
        ? planWarmRequest(body.messages, fullName)
        : { key: null, mode: 'fresh' };
      const warmResumeId = warmPlan.mode === 'resume' ? warmPlan.entry.conversationId : null;
      const initialConversationId = sessionInfo.conversationId || warmResumeId;
      const isResuming = Boolean(initialConversationId);
      const prompt = warmResumeId
        ? warmPlan.prompt
        : isResuming
          ? extractLatestUserPrompt(body.messages)
          : formatPrompt(body.messages);

      if (!prompt.trim()) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: 'Messages list produced empty prompt' }));
        return;
      }

      const reqId = `chatcmpl-${crypto.randomUUID()}`;
      const created = Math.floor(Date.now() / 1000);

      const runAgy = (targetConversationId, promptText, onComplete) => {
        const agyArgs = [
          '--dangerously-skip-permissions',
          '--disable-slash-commands'
        ];

        if (targetConversationId) {
          agyArgs.push('--conversation', targetConversationId);
        }

        agyArgs.push(
          '-p', promptText,
          '--model', fullName,
          '--output-format', stream ? 'stream-json' : 'json'
        );

        const child = spawn(AGY_BIN, agyArgs, {
          cwd: '/tmp',
          detached: true,
          env: {
            ...process.env,
            HOME: AGY_HOME
          },
          stdio: ['ignore', 'pipe', 'pipe']
        });

        onComplete(child);
      };

      let currentChild = null;
      let activeConversationId = initialConversationId || null;

      res.on('close', () => {
        if (!res.writableEnded && currentChild) {
          try {
            if (currentChild.pid) process.kill(-currentChild.pid, 'SIGTERM');
          } catch {
            try { currentChild.kill('SIGTERM'); } catch {}
          }
        }
      });

      const recordConversationId = (id) => {
        if (!id) return;
        activeConversationId = id;
        if (sessionInfo.key) {
          sessionStore.set(sessionInfo.key, { conversationId: id, updatedAt: Date.now() });
          pruneSessionStore();
        }
        if (!res.headersSent) {
          try {
            res.setHeader('X-Conversation-Id', id);
            res.setHeader('X-Session-Id', id);
          } catch {}
        }
      };

      if (stream) {
        res.writeHead(200, {
          'Content-Type': 'text/event-stream',
          'Cache-Control': 'no-cache',
          'Connection': 'keep-alive',
          'X-Accel-Buffering': 'no'
        });

        const executeStream = (targetConvId, promptToUse, allowRetry) => {
          let lineBuffer = '';
          let finalUsage = null;
          let responseEnded = false;
          let hasOutput = false;
          let stderrBuffer = '';
          let streamedText = '';

          runAgy(targetConvId, promptToUse, (child) => {
            currentChild = child;

            const processJsonLine = (line) => {
              if (!line.trim()) return;
              try {
                const obj = JSON.parse(line);
                if (obj.conversation_id) {
                  recordConversationId(obj.conversation_id);
                }
                if (obj.event === 'init' && obj.conversation_id) {
                  recordConversationId(obj.conversation_id);
                }
                if (obj.event === 'step_update' && obj.step_update?.text_delta) {
                  hasOutput = true;
                  streamedText += obj.step_update.text_delta;
                  const delta = obj.step_update.text_delta;
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
                } else if (obj.event === 'result') {
                  finalUsage = obj.result?.usage || null;
                  if (!streamedText && typeof obj.result?.response === 'string') {
                    streamedText = obj.result.response;
                  }
                  if (obj.result?.conversation_id) {
                    recordConversationId(obj.result.conversation_id);
                  }
                }
              } catch {}
            };

            child.stdout.on('data', chunk => {
              lineBuffer += chunk.toString();
              const lines = lineBuffer.split('\n');
              lineBuffer = lines.pop() || '';
              for (const line of lines) {
                processJsonLine(line);
              }
            });

            child.stderr.on('data', chunk => {
              stderrBuffer += chunk.toString();
            });

            child.on('close', (code, signal) => {
              if (responseEnded) return;

              if (lineBuffer.trim()) {
                processJsonLine(lineBuffer.trim());
              }

              // A resumed id the CLI does not know is silently degraded
              // to a fresh conversation holding only the new turn (the
              // CLI warns on stderr and carries on) — that is a resume
              // failure even when the exit code is 0.
              const resumeNotFound = Boolean(targetConvId)
                && /conversation .* not found/.test(stderrBuffer);

              // Fallback retry if resuming failed without output (output
              // already on the wire cannot be unsent, so a streamed
              // partial failure stays an error, per #678).
              if ((code !== 0 || resumeNotFound) && !hasOutput && allowRetry && targetConvId) {
                console.log(`Resuming conversation ${targetConvId} failed (code ${code}), retrying as fresh conversation...`);
                invalidateWarm(warmPlan.key);
                executeStream(null, formatPrompt(body.messages), false);
                return;
              }

              responseEnded = true;

              if (resumeNotFound && targetConvId) {
                // Streamed answer came from a degraded fresh conversation
                // that never saw the history: do not key it as fed.
                invalidateWarm(warmPlan.key);
              }

              // Failure is judged by the child's exit: a non-zero code or
              // a killing signal means the run failed, however much content
              // was already produced (#678). Partial output must not mask
              // a failure — it used to be reported as a successful 'stop'
              // carrying the truncated text (or an empty 200 when nothing
              // was produced at all), and stderrBuffer was never read.
              if (code !== 0) {
                const stderrLine = stderrBuffer.trim().split('\n').pop();
                const fallback = signal
                  ? `agy process terminated by signal ${signal}`
                  : `agy process exited with code ${code}`;
                const detail = (stderrLine || fallback).trim();
                const errorBody = { error: { message: `agy execution failed: ${detail}` } };
                // The 200 + text/event-stream headers are already on the
                // wire, so frame the error as an SSE event and close the
                // stream with [DONE] (mirrors muse-proxy #670).
                res.write(`data: ${JSON.stringify(errorBody)}\n\n`);
                res.write('data: [DONE]\n\n');
                res.end();
                return;
              }

              if (!resumeNotFound) {
                commitWarm(warmPlan.key, activeConversationId, body.messages, streamedText);
              }

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
                  prompt_tokens: finalUsage?.input_tokens || 0,
                  completion_tokens: finalUsage?.output_tokens || 0,
                  total_tokens: finalUsage?.total_tokens || 0
                }
              };

              res.write(`data: ${JSON.stringify(finishChunk)}\n\n`);
              res.write('data: [DONE]\n\n');
              res.end();
            });

            child.on('error', err => {
              if (!responseEnded) {
                responseEnded = true;
                res.write(`data: {"error": ${JSON.stringify(err.message)}}\n\n`);
                res.write('data: [DONE]\n\n');
                res.end();
              }
            });
          });
        };

        executeStream(initialConversationId, prompt, Boolean(initialConversationId));

      } else {
        // Non-streaming JSON mode
        const executeJson = (targetConvId, promptToUse, allowRetry) => {
          let stdoutBuffer = '';
          let stderrBuffer = '';

          runAgy(targetConvId, promptToUse, (child) => {
            currentChild = child;

            child.stdout.on('data', chunk => { stdoutBuffer += chunk.toString(); });
            child.stderr.on('data', chunk => { stderrBuffer += chunk.toString(); });

            child.on('close', (code, signal) => {
              if (res.writableEnded) return;

              let responseText = '';
              let usage = { input_tokens: 0, output_tokens: 0, total_tokens: 0 };
              let detectedConvId = null;

              try {
                const parsed = JSON.parse(stdoutBuffer.trim());
                responseText = parsed.response || '';
                if (parsed.usage) usage = parsed.usage;
                if (parsed.conversation_id) detectedConvId = parsed.conversation_id;
              } catch {
                responseText = stdoutBuffer.trim();
              }

              if (detectedConvId) {
                recordConversationId(detectedConvId);
              }

              // A resumed id the CLI does not know is silently degraded
              // to a fresh conversation holding only the new turn (the
              // CLI warns on stderr and carries on) — that is a resume
              // failure even when the exit code is 0.
              const resumeNotFound = Boolean(targetConvId)
                && /conversation .* not found/.test(stderrBuffer);

              // Failure is judged by the child's exit, not by whether it
              // produced output (#678): a run that fails after a partial
              // response must not be reported as a successful 'stop'
              // carrying the truncated text. A failed RESUME is the one
              // exception: nothing has been sent yet in JSON mode, so it
              // always falls back to a full-history fresh run and still
              // answers (#480) instead of surfacing the partial failure.
              if (code !== 0 || resumeNotFound) {
                if (allowRetry && targetConvId) {
                  console.log(`Resuming conversation ${targetConvId} failed (code ${code}), retrying as fresh conversation...`);
                  invalidateWarm(warmPlan.key);
                  executeJson(null, formatPrompt(body.messages), false);
                  return;
                }

                if (code !== 0) {
                  res.writeHead(502, { 'Content-Type': 'application/json' });
                  res.end(JSON.stringify({
                    error: 'agy process exited with non-zero code',
                    code,
                    ...(signal ? { signal } : {}),
                    stderr: stderrBuffer.trim()
                  }));
                  return;
                }
                // code === 0 with an unknown resume id and no retry left
                // cannot happen (allowRetry mirrors targetConvId); fall
                // through and answer with what the run produced.
              }

              commitWarm(warmPlan.key, activeConversationId, body.messages, responseText);

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
                      content: responseText
                    },
                    finish_reason: 'stop'
                  }
                ],
                usage: {
                  prompt_tokens: usage.input_tokens || 0,
                  completion_tokens: usage.output_tokens || 0,
                  total_tokens: usage.total_tokens || 0
                }
              }, null, 2));
            });

            child.on('error', err => {
              if (!res.writableEnded) {
                res.writeHead(500, { 'Content-Type': 'application/json' });
                res.end(JSON.stringify({ error: err.message }));
              }
            });
          });
        };

        executeJson(initialConversationId, prompt, Boolean(initialConversationId));
      }
    });
    return;
  }

  res.writeHead(404, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ error: 'Endpoint not found', path: pathname }));
});

server.listen(PORT, HOST, () => {
  publishEndpoint();
  console.log(`Antigravity agy Proxy listening on http://${HOST}:${PORT}`);
  console.log(`Endpoints:`);
  console.log(`  - GET  http://${HOST}:${PORT}/health`);
  console.log(`  - GET  http://${HOST}:${PORT}/v1/models`);
  console.log(`  - POST http://${HOST}:${PORT}/v1/chat/completions`);
});
