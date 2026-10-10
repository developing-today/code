#!/usr/bin/env node
import http from 'node:http';
import { spawn, execSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';

const PORT = parseInt(process.env.ANTIGRAVITY_LS_PROXY_PORT || '18745', 10);
const HOST = process.env.ANTIGRAVITY_LS_PROXY_HOST || '127.0.0.1';

const DEFAULT_MANAGED_HTTP_PORT = 48999;
const DEFAULT_MANAGED_HTTPS_PORT = 48998;
const MANAGED_CSRF_TOKEN = 'antigravity-ls-local-token-' + crypto.randomUUID().slice(0, 8);

const MODEL_MAP = {
  // Gemini 3.8 Flash
  'gemini-3.8-flash': { low: 'MODEL_PLACEHOLDER_M320', medium: 'MODEL_PLACEHOLDER_M319', high: 'MODEL_PLACEHOLDER_M318', default: 'medium', name: 'Gemini 3.8 Flash' },
  'gemini-3.8-flash-low': { enum: 'MODEL_PLACEHOLDER_M320', name: 'Gemini 3.8 Flash (Low)' },
  'gemini-3.8-flash-medium': { enum: 'MODEL_PLACEHOLDER_M319', name: 'Gemini 3.8 Flash (Medium)' },
  'gemini-3.8-flash-high': { enum: 'MODEL_PLACEHOLDER_M318', name: 'Gemini 3.8 Flash (High)' },

  // Gemini 3.7 Flash
  'gemini-3.7-flash': { low: 'MODEL_PLACEHOLDER_M300', medium: 'MODEL_PLACEHOLDER_M299', high: 'MODEL_PLACEHOLDER_M298', default: 'medium', name: 'Gemini 3.7 Flash' },
  'gemini-3.7-flash-low': { enum: 'MODEL_PLACEHOLDER_M300', name: 'Gemini 3.7 Flash (Low)' },
  'gemini-3.7-flash-medium': { enum: 'MODEL_PLACEHOLDER_M299', name: 'Gemini 3.7 Flash (Medium)' },
  'gemini-3.7-flash-high': { enum: 'MODEL_PLACEHOLDER_M298', name: 'Gemini 3.7 Flash (High)' },

  // Gemini 3.6 Flash
  'gemini-3.6-flash': { low: 'MODEL_PLACEHOLDER_M73', medium: 'MODEL_PLACEHOLDER_M72', high: 'MODEL_PLACEHOLDER_M71', default: 'medium', name: 'Gemini 3.6 Flash' },
  'gemini-3.6-flash-low': { enum: 'MODEL_PLACEHOLDER_M73', name: 'Gemini 3.6 Flash (Low)' },
  'gemini-3.6-flash-medium': { enum: 'MODEL_PLACEHOLDER_M72', name: 'Gemini 3.6 Flash (Medium)' },
  'gemini-3.6-flash-high': { enum: 'MODEL_PLACEHOLDER_M71', name: 'Gemini 3.6 Flash (High)' },

  // Gemini 3.1 Pro
  'gemini-3.1-pro': { low: 'MODEL_PLACEHOLDER_M36', high: 'MODEL_PLACEHOLDER_M37', default: 'high', name: 'Gemini 3.1 Pro' },
  'gemini-3.1-pro-low': { enum: 'MODEL_PLACEHOLDER_M36', name: 'Gemini 3.1 Pro (Low)' },
  'gemini-3.1-pro-high': { enum: 'MODEL_PLACEHOLDER_M37', name: 'Gemini 3.1 Pro (High)' },

  // Claude Sonnet 5.5
  'claude-sonnet-5-5': { low: 'MODEL_PLACEHOLDER_M403', medium: 'MODEL_PLACEHOLDER_M404', high: 'MODEL_PLACEHOLDER_M405', default: 'medium', name: 'Claude Sonnet 5.5' },
  'claude-sonnet-5-5-low': { enum: 'MODEL_PLACEHOLDER_M403', name: 'Claude Sonnet 5.5 (Low)' },
  'claude-sonnet-5-5-medium': { enum: 'MODEL_PLACEHOLDER_M404', name: 'Claude Sonnet 5.5 (Medium)' },
  'claude-sonnet-5-5-high': { enum: 'MODEL_PLACEHOLDER_M405', name: 'Claude Sonnet 5.5 (High)' },

  // Claude Opus 5.5
  'claude-opus-5-5': { low: 'MODEL_PLACEHOLDER_M400', medium: 'MODEL_PLACEHOLDER_M401', high: 'MODEL_PLACEHOLDER_M402', default: 'medium', name: 'Claude Opus 5.5' },
  'claude-opus-5-5-low': { enum: 'MODEL_PLACEHOLDER_M400', name: 'Claude Opus 5.5 (Low)' },
  'claude-opus-5-5-medium': { enum: 'MODEL_PLACEHOLDER_M401', name: 'Claude Opus 5.5 (Medium)' },
  'claude-opus-5-5-high': { enum: 'MODEL_PLACEHOLDER_M402', name: 'Claude Opus 5.5 (High)' },

  // GPT-OSS 120B
  'gpt-oss-120b': { enum: 'MODEL_OPENAI_GPT_OSS_120B_MEDIUM', name: 'GPT-OSS 120B' },
  'gpt-oss-120b-medium': { enum: 'MODEL_OPENAI_GPT_OSS_120B_MEDIUM', name: 'GPT-OSS 120B (Medium)' },

  // Direct Google Gemini fallback
  'gemini-2.5-flash': { enum: 'MODEL_GOOGLE_GEMINI_2_5_FLASH', name: 'Gemini 2.5 Flash' },
  'gemini-2.5-pro': { enum: 'MODEL_GOOGLE_GEMINI_2_5_PRO', name: 'Gemini 2.5 Pro' }
};

const resolveModelEnum = (requestedModel, requestedEffort) => {
  let modelStr = (requestedModel || 'gemini-3.8-flash').toLowerCase().trim();
  let isCascade = false;

  if (modelStr.endsWith('-cascade')) {
    isCascade = true;
    modelStr = modelStr.slice(0, -'-cascade'.length);
  }

  // Exact match
  if (MODEL_MAP[modelStr]?.enum) {
    return {
      modelEnum: MODEL_MAP[modelStr].enum,
      displayName: MODEL_MAP[modelStr].name,
      isCascade
    };
  }

  // Model family with effort
  const family = MODEL_MAP[modelStr];
  if (family) {
    let effort = (requestedEffort || family.default || 'medium').toLowerCase().trim();
    if (!family[effort]) effort = family.default || Object.keys(family)[0];
    const enumVal = family[effort];
    return {
      modelEnum: enumVal,
      displayName: `${family.name} (${effort.charAt(0).toUpperCase() + effort.slice(1)})`,
      isCascade
    };
  }

  // Direct pass-through if enum name or fallback
  if (modelStr.startsWith('model_')) {
    return { modelEnum: modelStr.toUpperCase(), displayName: modelStr, isCascade };
  }

  // Default to Gemini 3.8 Flash Medium
  return {
    modelEnum: 'MODEL_PLACEHOLDER_M319',
    displayName: 'Gemini 3.8 Flash (Medium)',
    isCascade
  };
};

const findLanguageServerBinary = () => {
  const candidates = [
    '/nix/store/rpgwxqadiscnsm9zmb4j4brb3yp2ayqg-antigravity-hub-2.12.2/share/antigravity/resources/bin/language_server',
    '/nix/store/4lcvb3j5418q7nlfx156b3rmf0vhv8qk-antigravity-ide-2.5.5/lib/antigravity-ide/resources/app/extensions/antigravity/bin/language_server_linux_x64'
  ];
  for (const c of candidates) {
    if (fs.existsSync(c)) return c;
  }
  try {
    const found = execSync('find /nix/store -name "language_server" -type f -executable 2>/dev/null | head -n 1', { encoding: 'utf-8' }).trim();
    if (found && fs.existsSync(found)) return found;
  } catch {}
  return candidates[0];
};

class LanguageServerManager {
  constructor() {
    this.managedChild = null;
    this.activeHttpPort = null;
    this.activeCsrfToken = null;
    this.isManaged = false;
  }

  async checkHeartbeat(port, token) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/exa.language_server_pb.LanguageServerService/Heartbeat`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'x-codeium-csrf-token': token
        },
        body: '{}',
        signal: AbortSignal.timeout(1500)
      });
      return res.status === 200;
    } catch {
      return false;
    }
  }

  detectRunningInstance() {
    if (process.env.ANTIGRAVITY_LS_ADDRESS && process.env.ANTIGRAVITY_CSRF_TOKEN) {
      const parts = process.env.ANTIGRAVITY_LS_ADDRESS.split(':');
      const port = parseInt(parts[parts.length - 1], 10);
      return { port, token: process.env.ANTIGRAVITY_CSRF_TOKEN };
    }

    try {
      const psOutput = execSync('ps aux | grep language_server | grep -v grep', { encoding: 'utf-8' });
      for (const line of psOutput.split('\n')) {
        if (!line.includes('language_server')) continue;
        const csrfMatch = line.match(/--csrf_token\s+([a-zA-Z0-9_-]+)/);
        if (!csrfMatch) continue;
        const token = csrfMatch[1];

        const portMatch = line.match(/--http_server_port\s+(\d+)/);
        if (portMatch && parseInt(portMatch[1], 10) > 0) {
          return { port: parseInt(portMatch[1], 10), token };
        }

        try {
          const logContent = fs.readFileSync(`${process.env.HOME}/.config/Antigravity/logs/language_server.log`, 'utf-8');
          const lines = logContent.split('\n').reverse().slice(0, 100);
          for (const l of lines) {
            const m = l.match(/listening on random port at (\d+) for HTTP/);
            if (m) {
              return { port: parseInt(m[1], 10), token };
            }
          }
        } catch {}
      }
    } catch {}

    return null;
  }

  async ensureActive() {
    if (this.activeHttpPort && this.activeCsrfToken) {
      const healthy = await this.checkHeartbeat(this.activeHttpPort, this.activeCsrfToken);
      if (healthy) return { port: this.activeHttpPort, token: this.activeCsrfToken };
    }

    const detected = this.detectRunningInstance();
    if (detected) {
      const healthy = await this.checkHeartbeat(detected.port, detected.token);
      if (healthy) {
        console.log(`Connected to active Antigravity Language Server on port ${detected.port}`);
        this.activeHttpPort = detected.port;
        this.activeCsrfToken = detected.token;
        this.isManaged = false;
        return detected;
      }
    }

    return this.startManagedDaemon();
  }

  async startManagedDaemon() {
    const bin = findLanguageServerBinary();
    if (!fs.existsSync(bin)) {
      throw new Error(`Antigravity language_server binary not found at ${bin}`);
    }

    console.log(`Standing up dedicated Antigravity Language Server daemon (${bin})...`);

    const args = [
      '--standalone',
      '--override_ide_name', 'antigravity',
      '--subclient_type', 'hub',
      '--override_ide_version', '2.12.2',
      '--override_user_agent_name', 'antigravity',
      '--csrf_token', MANAGED_CSRF_TOKEN,
      '--http_server_port', String(DEFAULT_MANAGED_HTTP_PORT),
      '--https_server_port', String(DEFAULT_MANAGED_HTTPS_PORT),
      '--app_data_dir', 'antigravity',
      '--api_server_url', 'https://generativelanguage.googleapis.com',
      '--cloud_code_endpoint', 'https://daily-cloudcode-pa.googleapis.com'
    ];

    const child = spawn(bin, args, {
      cwd: '/tmp',
      detached: true,
      stdio: ['ignore', 'pipe', 'pipe']
    });

    this.managedChild = child;
    this.activeHttpPort = DEFAULT_MANAGED_HTTP_PORT;
    this.activeCsrfToken = MANAGED_CSRF_TOKEN;
    this.isManaged = true;

    child.on('exit', (code, sig) => {
      console.log(`Managed language_server exited with code ${code}, signal ${sig}`);
      this.managedChild = null;
      this.activeHttpPort = null;
    });

    for (let i = 0; i < 30; i++) {
      await new Promise(r => setTimeout(r, 200));
      const ok = await this.checkHeartbeat(DEFAULT_MANAGED_HTTP_PORT, MANAGED_CSRF_TOKEN);
      if (ok) {
        console.log(`Dedicated Language Server ready on port ${DEFAULT_MANAGED_HTTP_PORT}`);
        return { port: this.activeHttpPort, token: this.activeCsrfToken };
      }
    }

    throw new Error('Timed out waiting for managed Antigravity Language Server to initialize');
  }

  async callRpc(method, payload) {
    const { port, token } = await this.ensureActive();
    const url = `http://127.0.0.1:${port}/exa.language_server_pb.LanguageServerService/${method}`;

    const res = await fetch(url, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'x-codeium-csrf-token': token
      },
      body: JSON.stringify(payload)
    });

    if (!res.ok) {
      const errText = await res.text();
      throw new Error(`RPC ${method} returned HTTP ${res.status}: ${errText}`);
    }

    return await res.json();
  }

  cleanup() {
    if (this.managedChild) {
      try {
        if (this.managedChild.pid) process.kill(-this.managedChild.pid, 'SIGTERM');
      } catch {
        try { this.managedChild.kill('SIGTERM'); } catch {}
      }
    }
  }
}

const lsManager = new LanguageServerManager();

process.on('SIGINT', () => { lsManager.cleanup(); process.exit(0); });
process.on('SIGTERM', () => { lsManager.cleanup(); process.exit(0); });

// Session store for multi-turn conversations
const sessionStore = new Map();
const MAX_SESSIONS = 1000;

const pruneSessionStore = () => {
  if (sessionStore.size <= MAX_SESSIONS) return;
  const entries = Array.from(sessionStore.entries())
    .sort((a, b) => a[1].updatedAt - b[1].updatedAt);
  const toDelete = entries.slice(0, entries.length - MAX_SESSIONS);
  for (const [key] of toDelete) sessionStore.delete(key);
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
    const trimmed = explicitId.trim();
    const cached = sessionStore.get(trimmed);
    return { key: trimmed, explicit: true, cascadeId: cached?.cascadeId || (trimmed.includes('-') && trimmed.length === 36 ? trimmed : null) };
  }

  if (body.user && typeof body.user === 'string' && body.user.trim()) {
    const userKey = `user:${body.user.trim()}`;
    const cached = sessionStore.get(userKey);
    return { key: userKey, explicit: false, cascadeId: cached?.cascadeId || null };
  }

  if (Array.isArray(body.messages) && body.messages.length > 1) {
    const firstMsg = body.messages[0];
    const firstContent = typeof firstMsg?.content === 'string'
      ? firstMsg.content
      : JSON.stringify(firstMsg?.content || '');
    if (firstContent.length > 0) {
      const hash = crypto.createHash('sha256').update(firstContent).digest('hex').slice(0, 16);
      const sessionKey = `turn0:${hash}`;
      const cached = sessionStore.get(sessionKey);
      return { key: sessionKey, explicit: false, cascadeId: cached?.cascadeId || null };
    }
  }

  return { key: null, explicit: false, cascadeId: null };
};

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://${req.headers.host || HOST}`);
  const pathname = url.pathname.replace(/\/+$/, '') || '/';

  // Health
  if (req.method === 'GET' && (pathname === '/health' || pathname === '/')) {
    try {
      const active = await lsManager.ensureActive();
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({
        status: 'ok',
        service: 'antigravity-ls-proxy',
        port: PORT,
        languageServer: {
          port: active.port,
          managed: lsManager.isManaged,
          activeSessions: sessionStore.size
        }
      }, null, 2));
    } catch (err) {
      res.writeHead(503, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ status: 'error', error: err.message }));
    }
    return;
  }

  // Models catalog
  if (req.method === 'GET' && (pathname === '/v1/models' || pathname === '/models')) {
    const now = Math.floor(Date.now() / 1000);
    const data = [];

    const registered = new Set();
    for (const [key, val] of Object.entries(MODEL_MAP)) {
      if (val.name && !registered.has(key)) {
        registered.add(key);
        data.push({
          id: key,
          object: 'model',
          created: now,
          owned_by: 'antigravity-ls',
          display_name: val.name
        });
      }
    }

    try {
      const live = await lsManager.callRpc('GetAvailableModels', {});
      const liveModels = live?.response?.models || {};
      for (const [k, v] of Object.entries(liveModels)) {
        if (!registered.has(k)) {
          registered.add(k);
          data.push({
            id: k,
            object: 'model',
            created: now,
            owned_by: 'antigravity-ls',
            display_name: v.displayName || k,
            max_tokens: v.maxTokens || 65536,
            supports_thinking: Boolean(v.supportsThinking)
          });
        }
      }
    } catch {}

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

    req.on('end', async () => {
      let body;
      try {
        body = JSON.parse(bodyText);
      } catch {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: 'Invalid JSON request body' }));
        return;
      }

      const stream = Boolean(body.stream);
      const requestedEffort = body.reasoning_effort || body.effort || (body.options && (body.options.reasoning_effort || body.options.effort));
      const { modelEnum, displayName, isCascade } = resolveModelEnum(body.model, requestedEffort);

      const sessionInfo = getSessionKey(req, body);
      const useCascade = isCascade || Boolean(sessionInfo.cascadeId) || body.mode === 'cascade';

      const reqId = `chatcmpl-${crypto.randomUUID()}`;
      const created = Math.floor(Date.now() / 1000);

      try {
        if (useCascade) {
          // CASCADE TRAJECTORY MODE
          let cascadeId = sessionInfo.cascadeId;
          if (!cascadeId) {
            const startResp = await lsManager.callRpc('StartCascade', {
              workspaceUris: ['file:///home/user/code'],
              source: 'CORTEX_TRAJECTORY_SOURCE_CLI'
            });
            cascadeId = startResp.cascadeId;
          }

          if (sessionInfo.key) {
            sessionStore.set(sessionInfo.key, { cascadeId, updatedAt: Date.now() });
            pruneSessionStore();
          }

          const userPrompt = extractLatestUserPrompt(body.messages);
          await lsManager.callRpc('SendUserCascadeMessage', {
            cascadeId,
            items: [{ text: userPrompt }],
            cascadeConfig: {
              plannerConfig: {
                requestedModel: {
                  model: modelEnum
                }
              }
            }
          });

          // Poll for completion
          let completedResponse = null;
          let usageInfo = null;

          for (let attempt = 0; attempt < 120; attempt++) {
            await new Promise(r => setTimeout(r, 250));
            const trajectory = await lsManager.callRpc('GetCascadeTrajectorySteps', { cascadeId });
            const steps = trajectory?.steps || [];
            const lastStep = steps[steps.length - 1];

            if (lastStep?.errorMessage) {
              throw new Error(lastStep.errorMessage?.error?.shortError || 'Cascade step error');
            }

            if (lastStep?.type === 'CORTEX_STEP_TYPE_PLANNER_RESPONSE' && lastStep?.status === 'CORTEX_STEP_STATUS_DONE') {
              completedResponse = lastStep.plannerResponse?.response || lastStep.plannerResponse?.modifiedResponse || '';
              usageInfo = lastStep.metadata?.modelUsage || null;
              break;
            }
          }

          if (completedResponse === null) {
            throw new Error('Cascade execution timed out after 30 seconds');
          }

          const inputTokens = parseInt(usageInfo?.inputTokens || '0', 10);
          const outputTokens = parseInt(usageInfo?.outputTokens || '0', 10);

          if (stream) {
            res.writeHead(200, {
              'Content-Type': 'text/event-stream',
              'Cache-Control': 'no-cache',
              'Connection': 'keep-alive',
              'X-Conversation-Id': cascadeId,
              'X-Session-Id': cascadeId
            });

            const chunk = {
              id: reqId,
              object: 'chat.completion.chunk',
              created,
              model: body.model || 'gemini-3.8-flash',
              conversation_id: cascadeId,
              choices: [{ index: 0, delta: { content: completedResponse }, finish_reason: null }]
            };
            res.write(`data: ${JSON.stringify(chunk)}\n\n`);

            const finishChunk = {
              id: reqId,
              object: 'chat.completion.chunk',
              created,
              model: body.model || 'gemini-3.8-flash',
              conversation_id: cascadeId,
              choices: [{ index: 0, delta: {}, finish_reason: 'stop' }],
              usage: { prompt_tokens: inputTokens, completion_tokens: outputTokens, total_tokens: inputTokens + outputTokens }
            };
            res.write(`data: ${JSON.stringify(finishChunk)}\n\n`);
            res.write('data: [DONE]\n\n');
            res.end();
          } else {
            res.writeHead(200, {
              'Content-Type': 'application/json',
              'X-Conversation-Id': cascadeId,
              'X-Session-Id': cascadeId
            });
            res.end(JSON.stringify({
              id: reqId,
              object: 'chat.completion',
              created,
              model: body.model || 'gemini-3.8-flash',
              conversation_id: cascadeId,
              choices: [{
                index: 0,
                message: { role: 'assistant', content: completedResponse },
                finish_reason: 'stop'
              }],
              usage: { prompt_tokens: inputTokens, completion_tokens: outputTokens, total_tokens: inputTokens + outputTokens }
            }, null, 2));
          }

        } else {
          // FAST DIRECT RPC MODE (GetModelResponse)
          const prompt = formatPrompt(body.messages);
          const resp = await lsManager.callRpc('GetModelResponse', {
            model: modelEnum,
            prompt
          });

          const responseText = resp.response || '';
          const outTokens = Math.max(1, Math.round(responseText.length / 4));
          const inTokens = Math.max(1, Math.round(prompt.length / 4));

          const activeId = sessionInfo.key || reqId;

          if (stream) {
            res.writeHead(200, {
              'Content-Type': 'text/event-stream',
              'Cache-Control': 'no-cache',
              'Connection': 'keep-alive',
              'X-Conversation-Id': activeId,
              'X-Session-Id': activeId
            });

            // Stream chunks
            const words = responseText.split(/(?<=\s+)/);
            for (const word of words) {
              const chunk = {
                id: reqId,
                object: 'chat.completion.chunk',
                created,
                model: body.model || 'gemini-3.8-flash',
                conversation_id: activeId,
                choices: [{ index: 0, delta: { content: word }, finish_reason: null }]
              };
              res.write(`data: ${JSON.stringify(chunk)}\n\n`);
            }

            const finishChunk = {
              id: reqId,
              object: 'chat.completion.chunk',
              created,
              model: body.model || 'gemini-3.8-flash',
              conversation_id: activeId,
              choices: [{ index: 0, delta: {}, finish_reason: 'stop' }],
              usage: { prompt_tokens: inTokens, completion_tokens: outTokens, total_tokens: inTokens + outTokens }
            };
            res.write(`data: ${JSON.stringify(finishChunk)}\n\n`);
            res.write('data: [DONE]\n\n');
            res.end();
          } else {
            res.writeHead(200, {
              'Content-Type': 'application/json',
              'X-Conversation-Id': activeId,
              'X-Session-Id': activeId
            });
            res.end(JSON.stringify({
              id: reqId,
              object: 'chat.completion',
              created,
              model: body.model || 'gemini-3.8-flash',
              conversation_id: activeId,
              choices: [{
                index: 0,
                message: { role: 'assistant', content: responseText },
                finish_reason: 'stop'
              }],
              usage: { prompt_tokens: inTokens, completion_tokens: outTokens, total_tokens: inTokens + outTokens }
            }, null, 2));
          }
        }
      } catch (err) {
        if (!res.headersSent) {
          res.writeHead(500, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ error: err.message }));
        } else {
          res.write(`data: {"error": ${JSON.stringify(err.message)}}\n\n`);
          res.write('data: [DONE]\n\n');
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
  console.log(`Antigravity Language Server Proxy listening on http://${HOST}:${PORT}`);
  console.log(`Endpoints:`);
  console.log(`  - GET  http://${HOST}:${PORT}/health`);
  console.log(`  - GET  http://${HOST}:${PORT}/v1/models`);
  console.log(`  - POST http://${HOST}:${PORT}/v1/chat/completions`);
});
