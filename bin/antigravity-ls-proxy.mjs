#!/usr/bin/env node
import http from 'node:http';
import { spawn, execSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { parseProxyPort, parseAddressPortValue } from './proxy-port.mjs';

const PORT = parseProxyPort('ANTIGRAVITY_LS_PROXY_PORT', 18745);
const HOST = process.env.ANTIGRAVITY_LS_PROXY_HOST || '127.0.0.1';

// Publish the bound endpoint so other tools can find this proxy without
// knowing its port in advance: $XDG_STATE_HOME/<name>/endpoint.json
// (default ~/.local/state/<name>/endpoint.json), written on listen,
// removed on exit. This file already registers SIGTERM/SIGINT handlers
// (see below, calling lsManager.cleanup() then process.exit(0)), so this
// helper does not register duplicate signal handlers; the 'exit' cleanup
// it registers runs on that existing exit path.
const publishEndpoint = () => {
  try {
    const stateHome =
      process.env.XDG_STATE_HOME || path.join(os.homedir(), '.local', 'state');
    const dir = path.join(stateHome, 'antigravity-ls-proxy');
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
  } catch {}
};

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

const resolveModelEnum = (requestedModel, requestedEffort, liveModels = {}) => {
  let modelStr = (requestedModel || 'gemini-3.8-flash').toLowerCase().trim();
  let isCascade = false;

  if (modelStr.endsWith('-cascade')) {
    isCascade = true;
    modelStr = modelStr.slice(0, -'-cascade'.length);
  }

  // Strip prefixes like 'antigravity-ls/', 'antigravity-agy/', 'antigravity/', 'google/'
  for (const prefix of ['antigravity-ls/', 'antigravity-agy/', 'antigravity/', 'google/']) {
    if (modelStr.startsWith(prefix)) {
      modelStr = modelStr.slice(prefix.length);
      break;
    }
  }

  // Candidates to test: original and dot-normalized (e.g. claude-sonnet-5.5 -> claude-sonnet-5-5)
  const candIds = [modelStr];
  const dotNormalized = modelStr.replace(/\./g, '-');
  if (dotNormalized !== modelStr) candIds.push(dotNormalized);

  // 1. Exact match in liveModels
  for (const cand of candIds) {
    if (liveModels[cand]?.model) {
      return {
        modelEnum: liveModels[cand].model,
        displayName: liveModels[cand].displayName || cand,
        isCascade
      };
    }
  }

  // 2. Exact match in MODEL_MAP
  for (const cand of candIds) {
    if (MODEL_MAP[cand]?.enum) {
      return {
        modelEnum: MODEL_MAP[cand].enum,
        displayName: MODEL_MAP[cand].name,
        isCascade
      };
    }
  }

  // 3. Model family with effort in MODEL_MAP
  for (const cand of candIds) {
    const family = MODEL_MAP[cand];
    if (family) {
      // A family entry also carries metadata keys — `default` (an effort
      // name) and `name` (a display name) — whose values are not model
      // enums. Accept an effort only when it maps to an actual enum,
      // otherwise fall back to the family's default effort (#686).
      const isModelEnum = (v) => typeof v === 'string' && v.startsWith('MODEL_');
      let effort = (requestedEffort || family.default || 'medium').toLowerCase().trim();
      if (!isModelEnum(family[effort])) effort = family.default;
      if (!isModelEnum(family[effort])) {
        effort = Object.keys(family).find((k) => isModelEnum(family[k]));
      }
      const enumVal = family[effort];
      return {
        modelEnum: enumVal,
        displayName: `${family.name} (${effort.charAt(0).toUpperCase() + effort.slice(1)})`,
        isCascade
      };
    }
  }

  // 4. Try matching with effort suffix in liveModels (e.g. claude-sonnet-5-5-medium, gemini-3.8-flash-high)
  const effortOrder = requestedEffort ? [requestedEffort.toLowerCase().trim(), 'medium', 'high', 'low', 'extra-low'] : ['medium', 'high', 'low', 'extra-low'];
  for (const cand of candIds) {
    for (const eff of effortOrder) {
      const variant = liveModels[`${cand}-${eff}`];
      if (variant?.model) {
        return {
          modelEnum: variant.model,
          displayName: variant.displayName || `${cand} (${eff})`,
          isCascade
        };
      }
    }
  }

  // 5. Direct pass-through if enum name
  if (modelStr.startsWith('model_')) {
    return { modelEnum: modelStr.toUpperCase(), displayName: modelStr, isCascade };
  }

  // Refuse rather than quietly run a different model than the one asked for.
  const err = new Error(`Unknown model "${requestedModel}": not in the language server catalog or the proxy's model map`);
  err.status = 400;
  throw err;
};

const findLanguageServerBinary = () => {
  // Set by the Nix unit to the exact hub build, so nothing here goes stale
  // when the flake bumps antigravity-hub.
  if (process.env.ANTIGRAVITY_LS_BIN) return process.env.ANTIGRAVITY_LS_BIN;

  const candidates = [
    '/nix/store/rpgwxqadiscnsm9zmb4j4brb3yp2ayqg-antigravity-hub-2.12.2/share/antigravity/resources/bin/language_server',
    '/nix/store/4lcvb3j5418q7nlfx156b3rmf0vhv8qk-antigravity-ide-2.5.5/lib/antigravity-ide/resources/app/extensions/antigravity/bin/language_server_linux_x64'
  ];
  for (const c of candidates) {
    if (fs.existsSync(c)) return c;
  }
  // Scan store entry names rather than `find /nix/store`, which walks the whole
  // store synchronously and would freeze the proxy while it runs.
  try {
    for (const entry of fs.readdirSync('/nix/store')) {
      if (!entry.includes('-antigravity-hub-')) continue;
      const c = `/nix/store/${entry}/share/antigravity/resources/bin/language_server`;
      if (fs.existsSync(c)) return c;
    }
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

  // Every running language server that might be ours to attach to, newest
  // first. Its HTTP port is logged rather than passed as a flag, and an HTTPS
  // listener is logged the same way, so ensureActive heartbeats each candidate
  // and keeps the one that answers.
  detectRunningInstances() {
    if (process.env.ANTIGRAVITY_LS_ADDRESS && process.env.ANTIGRAVITY_CSRF_TOKEN) {
      const port = parseAddressPortValue('ANTIGRAVITY_LS_ADDRESS', process.env.ANTIGRAVITY_LS_ADDRESS);
      return [{ port, token: process.env.ANTIGRAVITY_CSRF_TOKEN }];
    }

    const candidates = [];
    const seen = new Set();
    const add = (port, token) => {
      const p = parseInt(port, 10);
      const key = `${p}:${token}`;
      if (p >= 1 && p <= 65535 && !seen.has(key)) {
        seen.add(key);
        candidates.push({ port: p, token });
      }
    };

    // 1. Detect from active running processes and their listening sockets
    try {
      const psOutput = execSync('ps -ef | grep language_server | grep -v grep', { encoding: 'utf-8' });
      for (const line of psOutput.split('\n')) {
        if (!line.includes('language_server')) continue;
        const csrfMatch = line.match(/--csrf_token\s+([a-zA-Z0-9_-]+)/);
        if (!csrfMatch) continue;
        const token = csrfMatch[1];

        const tokens = line.trim().split(/\s+/);
        const pid = tokens[1];

        const portMatch = line.match(/--http_server_port\s+(\d+)/);
        if (portMatch) add(parseInt(portMatch[1], 10), token);

        if (pid) {
          try {
            const ssOut = execSync(`ss -tlnp 2>/dev/null | grep "pid=${pid},"`, { encoding: 'utf-8' });
            for (const ssLine of ssOut.split('\n')) {
              const m = ssLine.match(/(?:127\.0\.0\.1|0\.0\.0\.0|\[::\]|localhost):(\d+)/);
              if (m) add(m[1], token);
            }
          } catch {}
          try {
            const lsofOut = execSync(`lsof -a -p ${pid} -iTCP -sTCP:LISTEN -P -n 2>/dev/null`, { encoding: 'utf-8' });
            for (const lsofLine of lsofOut.split('\n')) {
              const m = lsofLine.match(/:(\d+)\s+\(LISTEN\)/);
              if (m) add(m[1], token);
            }
          } catch {}
        }
      }
    } catch {}

    // 2. Fallback: Parse log files for logged HTTP ports
    try {
      const logContent = fs.readFileSync(`${process.env.HOME}/.config/Antigravity/logs/language_server.log`, 'utf-8');
      const loggedPorts = logContent.split('\n').reverse()
        .map(l => l.match(/listening on random port at (\d+) for HTTP\b/))
        .filter(Boolean)
        .map(m => parseInt(m[1], 10));
      for (const cand of [...candidates]) {
        for (const port of loggedPorts) add(port, cand.token);
      }
    } catch {}

    return candidates;
  }

  async ensureActive() {
    if (this.activeHttpPort && this.activeCsrfToken) {
      const healthy = await this.checkHeartbeat(this.activeHttpPort, this.activeCsrfToken);
      if (healthy) return { port: this.activeHttpPort, token: this.activeCsrfToken };
    }

    for (const candidate of this.detectRunningInstances()) {
      if (await this.checkHeartbeat(candidate.port, candidate.token)) {
        console.log(`Connected to active Antigravity Language Server on port ${candidate.port}`);
        this.activeHttpPort = candidate.port;
        this.activeCsrfToken = candidate.token;
        this.isManaged = false;
        return candidate;
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

// The language server's model catalog (id -> { model enum, displayName }),
// pulled LIVE from the server's GetAvailableModels RPC (live-model rule,
// 2026-10-28) and cached for 60s so each chat request doesn't re-fetch
// it. Last-known-good: a failed refresh returns the previous successful
// catalog (not an empty one), so a transient server outage never shrinks
// the resolvable set; only a proxy that has never reached the server
// resolves from the static MODEL_MAP floor alone.
let liveCatalogCache = { at: 0, models: {} };
const getLiveCatalog = async () => {
  if (Date.now() - liveCatalogCache.at < 60_000) return liveCatalogCache.models;
  try {
    const live = await lsManager.callRpc('GetAvailableModels', {});
    liveCatalogCache = { at: Date.now(), models: live?.response?.models || {} };
  } catch {
    // Keep { at } unchanged so the next request retries the live pull.
    return liveCatalogCache.models;
  }
  return liveCatalogCache.models;
};

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

const formatPrompt = (messages, tools = []) => {
  if (!Array.isArray(messages) || messages.length === 0) return '';
  if (messages.length === 1 && messages[0].role === 'user' && (!tools || tools.length === 0)) {
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

  if (Array.isArray(tools) && tools.length > 0) {
    const toolsDesc = tools.map(t => {
      const fn = t.function || t;
      return `- ${fn.name}: ${fn.description || ''} (parameters: ${JSON.stringify(fn.parameters || {})})`;
    }).join('\n');
    parts.push(`[Available Tools]\n${toolsDesc}\n\n[Tool Calling]\nWhen you need to use a tool, emit each call as a fenced block in exactly this format, with nothing else inside the block:\n\`\`\`tool_call\n{"name": "<tool name>", "arguments": {<arguments as a JSON object>}}\n\`\`\`\nEmit one block per call. Never describe a call in prose instead of emitting its block. Any text outside the blocks is treated as your reply text.`);
  }

  if (convoMessages.length > 0) {
    const convoText = convoMessages.map(m => {
      let role = 'User';
      if (m.role === 'assistant') role = 'Assistant';
      else if (m.role === 'tool') role = `Tool Result (${m.name || m.tool_call_id || 'tool'})`;
      else if (m.role === 'user') role = 'User';
      else role = m.role;

      let text = '';
      if (typeof m.content === 'string') {
        text = m.content;
      } else if (Array.isArray(m.content)) {
        text = m.content.map(c => c.text || JSON.stringify(c)).join('\n');
      } else if (m.content) {
        text = JSON.stringify(m.content);
      }

      if (Array.isArray(m.tool_calls) && m.tool_calls.length > 0) {
        const calls = m.tool_calls.map(tc => {
          const fn = tc.function || tc;
          return `[Calls Tool: ${fn.name} with args ${fn.arguments || '{}'}]`;
        }).join('\n');
        text = text ? `${text}\n${calls}` : calls;
      }

      return `${role}: ${text}`;
    }).join('\n\n');
    parts.push(convoText);
  }

  return parts.join('\n\n');
};

// --- Tool-call extraction ---------------------------------------------------
// The language server returns plain text, so models spell tool calls in
// text: the canonical ```tool_call fenced JSON formatPrompt asks for,
// Claude's <invoke> XML, or a fenced/bare JSON object. When a request
// carries tools, lift every recognized call whose name matches an offered
// tool into structured OpenAI tool_calls and strip the syntax from the
// reply text. Without this, clients that require structured tool_calls
// (e.g. OpenCode) receive the calls as inert prose and execute nothing.
const coerceArgValue = (value) => {
  if (typeof value !== 'string') return value;
  const trimmed = value.trim();
  if (!trimmed) return value;
  try {
    return JSON.parse(trimmed);
  } catch {
    return value;
  }
};

const extractToolCalls = (text, tools) => {
  if (!text || !Array.isArray(tools) || tools.length === 0) {
    return { content: text, toolCalls: [] };
  }
  const validNames = new Set(tools.map(t => (t.function || t).name).filter(Boolean));
  if (validNames.size === 0) return { content: text, toolCalls: [] };

  const calls = [];
  const spans = [];
  const addCall = (name, args, start, end) => {
    if (!validNames.has(name) || start < 0) return;
    calls.push({ name, arguments: args && typeof args === 'object' ? args : {} });
    spans.push([start, end]);
  };

  // Fenced blocks: ```tool_call {json} ``` — a plain ```json fence whose
  // object names an offered tool counts too.
  const fenceRe = /```(?:tool_call|json)?[ \t]*\n?([\s\S]*?)```/g;
  for (const m of text.matchAll(fenceRe)) {
    const inner = m[1].trim();
    if (!inner.startsWith('{')) continue;
    try {
      const obj = JSON.parse(inner);
      if (obj && typeof obj.name === 'string') {
        addCall(obj.name, obj.arguments ?? obj.args ?? {}, m.index, m.index + m[0].length);
      }
    } catch {}
  }

  // Claude-style XML: <invoke name="x"><parameter name="y">v</parameter></invoke>
  const invokeRe = /<invoke\s+name="([^"]+)"\s*>([\s\S]*?)<\/invoke>/g;
  for (const m of text.matchAll(invokeRe)) {
    const args = {};
    for (const p of m[2].matchAll(/<parameter\s+name="([^"]+)"\s*>([\s\S]*?)<\/parameter>/g)) {
      args[p[1]] = coerceArgValue(p[2].trim());
    }
    addCall(m[1], args, m.index, m.index + m[0].length);
  }

  // A reply that is one bare JSON object naming an offered tool.
  if (calls.length === 0) {
    const trimmed = text.trim();
    if (trimmed.startsWith('{') && trimmed.endsWith('}')) {
      try {
        const obj = JSON.parse(trimmed);
        if (obj && typeof obj.name === 'string' && validNames.has(obj.name)) {
          addCall(obj.name, obj.arguments ?? obj.args ?? {}, text.indexOf('{'), text.lastIndexOf('}') + 1);
        }
      } catch {}
    }
  }

  if (calls.length === 0) return { content: text, toolCalls: [] };

  spans.sort((a, b) => a[0] - b[0]);
  let content = '';
  let cursor = 0;
  for (const [start, end] of spans) {
    if (start < cursor) continue;
    content += text.slice(cursor, start);
    cursor = end;
  }
  content += text.slice(cursor);
  content = content.replace(/<function_calls>|<\/function_calls>/g, '').replace(/\n{3,}/g, '\n\n').trim();

  return {
    content,
    toolCalls: calls.map(c => ({
      id: `call_${crypto.randomUUID().replace(/-/g, '').slice(0, 24)}`,
      type: 'function',
      function: { name: c.name, arguments: JSON.stringify(c.arguments) }
    }))
  };
};

// Emit one streaming chunk per tool call (OpenAI delta.tool_calls shape).
const writeToolCallChunks = (res, base, toolCalls) => {
  toolCalls.forEach((tc, i) => {
    const chunk = {
      ...base,
      choices: [{
        index: 0,
        delta: {
          tool_calls: [{
            index: i,
            id: tc.id,
            type: 'function',
            function: { name: tc.function.name, arguments: tc.function.arguments }
          }]
        },
        finish_reason: null
      }]
    };
    res.write(`data: ${JSON.stringify(chunk)}\n\n`);
  });
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
      const liveModels = await getLiveCatalog();
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
      let resolved;
      try {
        resolved = resolveModelEnum(body.model, requestedEffort, await getLiveCatalog());
      } catch (err) {
        res.writeHead(err.status || 500, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: err.message }));
        return;
      }
      const { modelEnum, displayName, isCascade } = resolved;

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

          const cascadeExtract = extractToolCalls(completedResponse, body.tools);
          const cascadeContent = cascadeExtract.content;
          const cascadeToolCalls = cascadeExtract.toolCalls;
          const cascadeFinish = cascadeToolCalls.length > 0 ? 'tool_calls' : 'stop';
          const cascadeBase = {
            id: reqId,
            object: 'chat.completion.chunk',
            created,
            model: body.model || 'gemini-3.8-flash',
            conversation_id: cascadeId
          };

          if (stream) {
            res.writeHead(200, {
              'Content-Type': 'text/event-stream',
              'Cache-Control': 'no-cache',
              'Connection': 'keep-alive',
              'X-Conversation-Id': cascadeId,
              'X-Session-Id': cascadeId
            });

            if (cascadeContent) {
              const chunk = {
                ...cascadeBase,
                choices: [{ index: 0, delta: { content: cascadeContent }, finish_reason: null }]
              };
              res.write(`data: ${JSON.stringify(chunk)}\n\n`);
            }

            if (cascadeToolCalls.length > 0) writeToolCallChunks(res, cascadeBase, cascadeToolCalls);

            const finishChunk = {
              ...cascadeBase,
              choices: [{ index: 0, delta: {}, finish_reason: cascadeFinish }],
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
                message: {
                  role: 'assistant',
                  content: cascadeToolCalls.length > 0 ? (cascadeContent || null) : cascadeContent,
                  ...(cascadeToolCalls.length > 0 ? { tool_calls: cascadeToolCalls } : {})
                },
                finish_reason: cascadeFinish
              }],
              usage: { prompt_tokens: inputTokens, completion_tokens: outputTokens, total_tokens: inputTokens + outputTokens }
            }, null, 2));
          }

        } else {
          // FAST DIRECT RPC MODE (GetModelResponse)
          const prompt = formatPrompt(body.messages, body.tools);
          const resp = await lsManager.callRpc('GetModelResponse', {
            model: modelEnum,
            prompt
          });

          const responseText = resp.response || '';
          const outTokens = Math.max(1, Math.round(responseText.length / 4));
          const inTokens = Math.max(1, Math.round(prompt.length / 4));

          const extracted = extractToolCalls(responseText, body.tools);
          const finalContent = extracted.content;
          const toolCalls = extracted.toolCalls;
          const finishReason = toolCalls.length > 0 ? 'tool_calls' : 'stop';

          const activeId = sessionInfo.key || reqId;
          const directBase = {
            id: reqId,
            object: 'chat.completion.chunk',
            created,
            model: body.model || 'gemini-3.8-flash',
            conversation_id: activeId
          };

          if (stream) {
            res.writeHead(200, {
              'Content-Type': 'text/event-stream',
              'Cache-Control': 'no-cache',
              'Connection': 'keep-alive',
              'X-Conversation-Id': activeId,
              'X-Session-Id': activeId
            });

            // Stream chunks
            const words = finalContent ? finalContent.split(/(?<=\s+)/) : [];
            for (const word of words) {
              const chunk = {
                ...directBase,
                choices: [{ index: 0, delta: { content: word }, finish_reason: null }]
              };
              res.write(`data: ${JSON.stringify(chunk)}\n\n`);
            }

            if (toolCalls.length > 0) writeToolCallChunks(res, directBase, toolCalls);

            const finishChunk = {
              ...directBase,
              choices: [{ index: 0, delta: {}, finish_reason: finishReason }],
              usage: { prompt_tokens: inTokens, completion_tokens: outTokens, total_tokens: inTokens + outTokens }
            };
            res.write(`data: ${JSON.stringify(finishChunk)}\n\n`);

            if (body.stream_options?.include_usage) {
              const usageChunk = {
                id: reqId,
                object: 'chat.completion.chunk',
                created,
                model: body.model || 'gemini-3.8-flash',
                conversation_id: activeId,
                choices: [],
                usage: { prompt_tokens: inTokens, completion_tokens: outTokens, total_tokens: inTokens + outTokens }
              };
              res.write(`data: ${JSON.stringify(usageChunk)}\n\n`);
            }

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
                message: {
                  role: 'assistant',
                  content: toolCalls.length > 0 ? (finalContent || null) : finalContent,
                  ...(toolCalls.length > 0 ? { tool_calls: toolCalls } : {})
                },
                finish_reason: finishReason
              }],
              usage: { prompt_tokens: inTokens, completion_tokens: outTokens, total_tokens: inTokens + outTokens }
            }, null, 2));
          }
        }
      } catch (err) {
        let status = err.status || 500;
        let errType = 'api_error';
        const msg = err.message || 'Internal server error';

        if (msg.includes('429') || msg.includes('RESOURCE_EXHAUSTED') || msg.includes('quota reached')) {
          status = 429;
          errType = 'insufficient_quota';
        } else if (msg.includes('Unknown model') || msg.includes('Invalid JSON') || status === 400) {
          status = 400;
          errType = 'invalid_request_error';
        }

        const errPayload = {
          error: {
            message: msg,
            type: errType,
            code: status
          }
        };

        if (!res.headersSent) {
          res.writeHead(status, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify(errPayload, null, 2));
        } else {
          res.write(`data: ${JSON.stringify(errPayload)}\n\n`);
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
  publishEndpoint();
  console.log(`Antigravity Language Server Proxy listening on http://${HOST}:${PORT}`);
  console.log(`Endpoints:`);
  console.log(`  - GET  http://${HOST}:${PORT}/health`);
  console.log(`  - GET  http://${HOST}:${PORT}/v1/models`);
  console.log(`  - POST http://${HOST}:${PORT}/v1/chat/completions`);
});
