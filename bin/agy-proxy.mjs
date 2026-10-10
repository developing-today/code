#!/usr/bin/env node
import http from 'node:http';
import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';

const PORT = parseInt(process.env.AGY_PROXY_PORT || '18743', 10);
const HOST = process.env.AGY_PROXY_HOST || '127.0.0.1';

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
      activeSessions: sessionStore.size
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
      let activeConversationId = sessionInfo.conversationId || null;

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

            child.on('close', code => {
              if (responseEnded) return;

              // Fallback retry if resuming failed without output
              if (code !== 0 && !hasOutput && allowRetry && targetConvId) {
                console.log(`Resuming conversation ${targetConvId} failed (code ${code}), retrying as fresh conversation...`);
                executeStream(null, formatPrompt(body.messages), false);
                return;
              }

              responseEnded = true;

              if (lineBuffer.trim()) {
                processJsonLine(lineBuffer.trim());
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

        executeStream(sessionInfo.conversationId, prompt, Boolean(sessionInfo.conversationId));

      } else {
        // Non-streaming JSON mode
        const executeJson = (targetConvId, promptToUse, allowRetry) => {
          let stdoutBuffer = '';
          let stderrBuffer = '';

          runAgy(targetConvId, promptToUse, (child) => {
            currentChild = child;

            child.stdout.on('data', chunk => { stdoutBuffer += chunk.toString(); });
            child.stderr.on('data', chunk => { stderrBuffer += chunk.toString(); });

            child.on('close', code => {
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

              if (code !== 0 && !responseText) {
                if (allowRetry && targetConvId) {
                  console.log(`Resuming conversation ${targetConvId} failed (code ${code}), retrying as fresh conversation...`);
                  executeJson(null, formatPrompt(body.messages), false);
                  return;
                }

                res.writeHead(502, { 'Content-Type': 'application/json' });
                res.end(JSON.stringify({
                  error: 'agy process exited with non-zero code',
                  code,
                  stderr: stderrBuffer.trim()
                }));
                return;
              }

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

        executeJson(sessionInfo.conversationId, prompt, Boolean(sessionInfo.conversationId));
      }
    });
    return;
  }

  res.writeHead(404, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ error: 'Endpoint not found', path: pathname }));
});

server.listen(PORT, HOST, () => {
  console.log(`Antigravity agy Proxy listening on http://${HOST}:${PORT}`);
  console.log(`Endpoints:`);
  console.log(`  - GET  http://${HOST}:${PORT}/health`);
  console.log(`  - GET  http://${HOST}:${PORT}/v1/models`);
  console.log(`  - POST http://${HOST}:${PORT}/v1/chat/completions`);
});
