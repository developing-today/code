#!/usr/bin/env node
// commandcode-proxy — OpenAI-compatible bridge to the CommandCode CLI.
//
// Why this exists: CommandCode's hosted API (api.commandcode.ai/provider/v1)
// rejects Drew's Go plan ("Your Go plan doesn't include API access. Upgrade
// to Provider or higher"), so the only working route is the CLI itself,
// exactly like bin/codex-proxy.mjs (Muse) and
// ~/tools/muse-proxy/muse-proxy.mjs (Muse CLI): one headless CLI
// subprocess per chat-completions request.
//
// CLI contract (v1.88.0, verified live 2026-10-24):
//   commandcode -p --output-format json -m <model> --max-turns 1
//     --no-session --skip-onboarding --no-auto-update -t
//     --permission-mode dont-ask
//   with the flattened conversation written to the child's STDIN
//   (`-p` with no query argument reads the prompt from stdin — no argv
//   length/escaping concerns). Stdout is NDJSON:
//     {"type":"event","event":{"type":"text_delta","delta":"..."}}
//     {"type":"event","event":{"type":"thinking_delta",...}}   (ignored)
//     {"type":"event","event":{"type":"run_end","result":
//       {"finalText":"...","usage":{"inputTokens":N,"outputTokens":N}}}}
//   Exit 0 on success, 8 on max-turns cap.
//
// Session mode shipped: RESIDENT ACP (phase 2) — see the "Resident ACP
// engine" section below. One `cmd acp` process is held by this service;
// conversations map to persistent ACP threads and per call only the
// newest user turn is sent. The warm-session spawn chain (resume-by-
// transcript, then fresh full-history) remains as the automatic
// fallback if the ACP path fails.
//
// Agency (2026-10-25): when the incoming OpenAI request carries `tools`
// (i.e. OpenCode runs an agentic turn — its agent loop only sends tools
// when it wants tool calls), the CLI runs tool-enabled and does the task
// with ITS OWN tools; the bridge returns its final text. Chat turns (no
// tools) keep the no-tools steer. Verified live: the CLI's ACP tools are
// internal (write_file/edit/shell against the session cwd), so no ACP
// client fs/terminal handlers are needed; session mode 'dont-ask' fails
// tools CLOSED with "Permission denied by the user" (no client prompt is
// sent), so agency threads stay on the default mode while chat threads
// are set to 'dont-ask'. 'yolo' is not a valid ACP modeId (Unknown mode)
// — only a CLI flag, used on the agency spawn fallback with --tools-all.
// The agency prompt confines file work to the per-conversation session
// workdir; the CLI process itself is unconfined (fleet parity with
// codex-proxy's danger-full-access).
//
// Auth is the CLI's own login (~/.commandcode/auth.json). No API key is
// needed or read; clients may send the placeholder "commandcode-local"
// (accepted and ignored, like the other fleet proxies' *-local keys).
import http from 'node:http';
import { spawn, execFile } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseProxyMillis, parseProxyPort } from './proxy-port.mjs';

const PORT = parseProxyPort('COMMANDCODE_PROXY_PORT', 18748);
const HOST = process.env.COMMANDCODE_PROXY_HOST || '127.0.0.1';
const TIMEOUT_MS = parseProxyMillis('COMMANDCODE_PROXY_TIMEOUT_MS', 240000);
const MAX_TURNS = process.env.COMMANDCODE_PROXY_MAX_TURNS || '1';
// Spawn-fallback turn budget for agency turns only (chat stays at
// MAX_TURNS). The ACP path has no turn cap (bounded by TIMEOUT_MS).
const AGENCY_MAX_TURNS = process.env.COMMANDCODE_PROXY_AGENCY_MAX_TURNS || '8';

const EGRESS_PROXY = process.env.COMMANDCODE_PROXY_EGRESS ?? 'http://hatch-egress-proxy:3128';
const EGRESS_CA = '/run/hatch/egress-tls/ca-bundle.pem';

const PROXY_DIR = path.dirname(fileURLToPath(import.meta.url));
const WORKROOT = process.env.COMMANDCODE_PROXY_WORKDIR || path.join(PROXY_DIR, 'work');
fs.mkdirSync(WORKROOT, { recursive: true });

// Static floor for the served model list (live-model rule, 2026-10-28):
// the ids below are only the last-resort fallback. The served list is
// pulled LIVE from `commandcode --list-models` (see refreshLiveModels
// below) with a TTL cache and last-known-good fallback, so a model the
// CLI gains or drops shows up here without a code change. This floor is
// the 2026-10-25 snapshot (85 models, CLI v1.88.0). typesafe/jev is
// deliberately omitted everywhere (including the live parse): it is a
// headless-only decision model taking a JSON state/questions payload,
// not a chat model. This is the documented-impossibility exception to
// the live-model rule, re-verified with evidence 2026-10-28:
//   - Upstream `commandcode --list-models` (CLI v1.88.0) segregates it
//     itself: the banner reads "Available models · 94 models" and jev
//     appears only afterwards, under a separate "Decision models
//     (headless only)" heading: "typed questions in, probabilities
//     out". Vendor docs (Available Models / Provider API / Headless)
//     agree: "not a chat model", "not available in /model and can't
//     drive a session", served on the Provider API only via a separate
//     `v1/systemone` endpoint because "its request is not a chat
//     message, and it never streams".
//   - Demonstrated live through this proxy (2026-10-28, while the CLI
//     login was expired): a chat-completions request for typesafe/jev
//     ("Reply with exactly: PROBE-OK") failed 502 with a jev-specific
//     error BEFORE any auth check — "a decision model takes a JSON
//     request, not a prompt. Expected: {\"state\": <text or JSON>,
//     \"questions\": {...}}" — while the identical request for
//     gpt-6.1-sol reached the upstream and failed only on auth
//     ("POST /alpha/generate → 401 error: Invalid 'Authorization'
//     header or token."). A direct CLI run with a well-formed JSON
//     state/questions payload reached the auth boundary instead
//     ("Authentication failed. Please run \"cmd login\" first."), so
//     the request SHAPE, not auth, is the discriminator.
// Serving jev in /v1/models would advertise it as a chat model this
// bridge cannot serve chat traffic for; callers that need jev must
// use the CLI headless JSON form or the Provider API systemone
// endpoint directly.
const STATIC_MODELS = [
  'deepseek/deepseek-v4-pro',
  'deepseek/deepseek-v4-flash',
  'deepseek/deepseek-v4-flash-vision-exp',
  'deepseek/deepseek-v4-flash-fast',
  'deepseek/deepseek-v4.1-flash',
  'deepseek/deepseek-v4.1-flash-fast',
  'moonshotai/kimi-k3',
  'moonshotai/kimi-k2.7-code',
  'moonshotai/kimi-k2.7-code-highspeed',
  'moonshotai/kimi-k2.6',
  'moonshotai/kimi-k2.5',
  'z-ai/glm-5.3-flash',
  'z-ai/glm-5.3-flashx',
  'zai-org/glm-5.3',
  'zai-org/glm-5.2',
  'zai-org/glm-5.2-fast',
  'zai-org/glm-5.1',
  'zai-org/glm-5',
  'minimaxai/minimax-m3',
  'minimaxai/minimax-m2.7',
  'minimaxai/minimax-m2.5',
  'xiaomi/mimo-v2.6-pro',
  'xiaomi/mimo-v2.6-pro-ultraspeed',
  'xiaomi/mimo-v2.6-flash',
  'xiaomi/mimo-v2.5-pro',
  'xiaomi/mimo-v2.5',
  'meituan/longcat-2.0',
  'qwen/qwen3.8-omni-flash',
  'qwen/qwen3.8-max-0902',
  'qwen/qwen3.8-max',
  'qwen/qwen3.8-27b',
  'qwen/qwen3.8-flash',
  'qwen/qwen3.7-max',
  'qwen/qwen3.7-plus',
  'qwen/qwen3.7-flash',
  'qwen/qwen3.6-max-preview',
  'qwen/qwen3.6-plus',
  'stepfun/step-5-preview',
  'stepfun/step-3.7-flash',
  'stepfun/step-3.5-flash',
  'tencent/hy3-paid',
  'tencent/hy4-preview',
  'nvidia/nemotron-3-ultra-550b-a55b',
  'thinkingmachines/inkling',
  'thinkingmachines/inkling-small',
  'poolside/laguna-s-2.1-free',
  'inclusionai/ling-3.1-flash:free',
  'inclusionai/ling-3.0-flash-sante:free',
  'stealth/space-bunny-alpha',
  'claude-sonnet-5-5',
  'claude-sonnet-5',
  'claude-sonnet-4-6',
  'claude-fable-5-1',
  'claude-fable-5',
  'claude-opus-5-5',
  'claude-opus-5',
  'claude-opus-4-8',
  'claude-opus-4-7',
  'claude-haiku-4-5',
  'gpt-6-astra',
  'gpt-6.1-sol',
  'gpt-6-sol',
  'gpt-6-luna',
  'gpt-5.6-sol',
  'gpt-5.6-terra',
  'gpt-5.6-luna',
  'gpt-5.5',
  'gpt-5.4',
  'gpt-5.3-codex',
  'gpt-5.4-mini',
  'google/gemini-3.8-flash',
  'google/gemini-3.7-flash',
  'google/gemini-3.6-flash',
  'google/gemini-3.5-flash',
  'google/gemini-3.5-flash-lite',
  'google/gemini-3.1-flash-lite',
  'sakana/fugu-ultra',
  'meta/muse-spark-1.1',
  'meta/muse-spark-1.2',
  'meta/muse-spark-1.2-contributor',
  'meta/muse-spark-1.3',
  'meta/muse-spark-1.3-contributor',
  'xai/grok-4.5',
  'xai/grok-4.6',
  'xai/grok-4.7'
];
const DEFAULT_MODEL = 'deepseek/deepseek-v4-flash';

// Publish the bound endpoint so other tools can find this proxy without
// knowing its port in advance: $XDG_STATE_HOME/<name>/endpoint.json
// (default ~/.local/state/<name>/endpoint.json), written on listen,
// removed on exit. (Copied from agy-proxy.)
const publishEndpoint = () => {
  try {
    const stateHome =
      process.env.XDG_STATE_HOME || path.join(os.homedir(), '.local', 'state');
    const dir = path.join(stateHome, 'commandcode-proxy');
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

const findCommandCodeBinary = () => {
  const candidates = [
    `${process.env.HOME}/tools/command-code/bin/commandcode`,
    `${process.env.HOME}/.local/bin/commandcode`,
    'commandcode'
  ];
  for (const c of candidates) {
    if (c === 'commandcode' || fs.existsSync(c)) {
      return c;
    }
  }
  return 'commandcode';
};

const CC_BIN = findCommandCodeBinary();

// --- Live model list (2026-10-28 standing rule: pull live, never ship a
// static list as the source of truth) -------------------------------------
// `commandcode --list-models` is authoritative but slow (~28s cold), so
// the pull is asynchronous: requests serve the last-known-good list (or
// the static floor before the first successful pull) and a stale entry
// triggers a background refresh (stale-while-revalidate). TTL 5 min.
const MODEL_LIST_TTL_MS = 5 * 60 * 1000;
let liveModelCache = { at: 0, models: null };
let liveModelRefresh = null;

const parseListModelsOutput = (text) => {
  const ids = [];
  for (const line of String(text).split('\n')) {
    const token = line.trim().split(/\s+/)[0] || '';
    // Model ids are vendor/name, claude-* or gpt-*; section headers and
    // the "Available models" banner never match this shape.
    if (!token.includes('/') && !token.startsWith('claude-') && !token.startsWith('gpt-')) continue;
    if (token === 'typesafe/jev') continue; // headless-only, see floor note
    if (!ids.includes(token)) ids.push(token);
  }
  return ids;
};

const refreshLiveModels = () => {
  if (liveModelRefresh) return liveModelRefresh;
  liveModelRefresh = new Promise((resolve) => {
    execFile(CC_BIN, ['--list-models'], {
      timeout: 120000, maxBuffer: 8 * 1024 * 1024, input: '', env: childEnv()
    }, (err, stdout) => {
      liveModelRefresh = null;
      if (!err) {
        const ids = parseListModelsOutput(stdout);
        // Sanity floor: a garbled/truncated pull must not shrink the
        // served list to near-empty; keep the last-known-good instead.
        if (ids.length >= 20) {
          liveModelCache = { at: Date.now(), models: ids };
        } else {
          console.warn(`[commandcode-proxy] --list-models returned only ${ids.length} ids; keeping last-known-good`);
        }
      } else {
        console.warn(`[commandcode-proxy] --list-models refresh failed (${err.message}); serving last-known-good`);
      }
      resolve(liveModelCache.models);
    });
  });
  return liveModelRefresh;
};

const getServedModels = () => {
  if (!liveModelCache.models || Date.now() - liveModelCache.at > MODEL_LIST_TTL_MS) {
    refreshLiveModels(); // fire-and-forget; serve cache/floor meanwhile
  }
  return liveModelCache.models || STATIC_MODELS;
};

const resolveModel = (requestedModel) => {
  let raw = (requestedModel || '').trim();
  if (!raw) return DEFAULT_MODEL;
  // OpenCode may send the id bare; a stray leading "commandcode/" is the
  // provider name, not part of the model id — but model ids themselves
  // contain slashes (deepseek/...), so only strip that exact prefix.
  if (raw.startsWith('commandcode/')) raw = raw.slice('commandcode/'.length);
  const served = getServedModels();
  if (served.includes(raw)) return raw;
  // CLI also accepts the short name after the last '/'.
  const byShort = served.find(m => m.slice(m.lastIndexOf('/') + 1) === raw);
  if (byShort) return byShort;
  // Unknown id: pass through — the CLI may know a newer model than this list.
  return raw;
};

const formatPrompt = (messages, agency, workdir, toolManifest) => {
  if (!Array.isArray(messages) || messages.length === 0) return '';
  if (!agency && messages.length === 1 && messages[0].role === 'user' && typeof messages[0].content === 'string') {
    return messages[0].content;
  }

  const parts = [];
  if (agency) {
    // Agency turn (the OpenAI request carried tools, i.e. OpenCode wants
    // an agentic turn): the CLI does the task with ITS OWN tools and this
    // bridge returns its final text. Verified live 2026-10-25: the CLI's
    // ACP tools are internal (write_file/edit/shell against the session
    // cwd) — no ACP client fs/terminal handlers are needed. Permission
    // posture is the session mode: 'dont-ask' FAILS tools closed
    // ("Permission denied by the user", no client prompt is sent), so
    // agency threads stay on the default mode and this proxy auto-allows
    // any session/request_permission the CLI does send (see handleLine).
    // Keep all file work inside the assigned workdir (absolute paths).
    parts.push(`[Bridge instruction: act as the tool-using agent for the conversation below. Use your own file/shell tools to complete the task. Do ALL file work inside ${workdir} (absolute paths under it); do not touch anything outside it. When done, reply with a brief text summary of what you did. OpenCode asked for these tools — implement each with your closest equivalent: ${toolManifest}]`);
  } else {
  // The CLI is an agentic coding tool with its own system prompt; without
  // this steer it treats the flattened conversation as a workspace task
  // and burns multiple turns on tool calls (observed: a trivial
  // "Reply with exactly: PROBE-OK" from OpenCode took ~208s at
  // max-turns 5). This bridge is a chat completion: one direct text
  // answer, no tools, no workspace exploration.
  parts.push('[Bridge instruction: answer the conversation below directly with a single text reply. Do not invoke tools, do not explore or modify any workspace, do not run commands.]');
  }
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

// Child env: inherit, and pin the egress proxy + CA bundle the CLI needs
// to reach CommandCode from the sandbox (same treatment as muse-proxy).
// NODE_USE_ENV_PROXY=1 is REQUIRED (verified 2026-10-24): the CLI is a
// Node app whose fetch() ignores *_PROXY env vars without it, tries a
// direct connection, and dies with run_error TransportError
// "POST /alpha/generate → network error: fetch failed" / stderr
// "Error: Unable to connect to the API". With it set, the
// unauthenticated egress proxy works.
const childEnv = () => {
  const env = { ...process.env };
  env.NODE_USE_ENV_PROXY = '1';
  if (EGRESS_PROXY) {
    env.HTTPS_PROXY = EGRESS_PROXY;
    env.HTTP_PROXY = EGRESS_PROXY;
    env.https_proxy = EGRESS_PROXY;
    env.http_proxy = EGRESS_PROXY;
    env.ALL_PROXY = EGRESS_PROXY;
    env.all_proxy = EGRESS_PROXY;
  }
  if (EGRESS_PROXY && fs.existsSync(EGRESS_CA)) env.SSL_CERT_FILE = EGRESS_CA;
  env.NO_PROXY = '127.0.0.1,localhost';
  env.no_proxy = '127.0.0.1,localhost';
  return env;
};


// ---------------------------------------------------------------------------
// Warm-session support (fleet design: developing-today/code issue #480).
//
// The CommandCode CLI persists sessions under
// ~/.commandcode/projects/<slug-of-cwd>/<sessionId>.jsonl (transcript),
// <sessionId>.checkpoints.jsonl and <sessionId>.meta.json, and
// `commandcode -p --session <id|unique-prefix>` resumes one from any cwd
// (verified live 2026-10-24: a codeword stored in turn 1 was recalled in
// turn 2 from only the new turn's prompt, with usage cacheReadTokens
// 19840 of 19934 input — ~99% of the replayed context served from the
// provider prompt cache). So when a conversation key is known, the proxy
// resumes the mapped CLI session and sends ONLY the newest user turn
// instead of re-flattening the whole history; any resume failure falls
// back to a full-history fresh run (which re-creates the mapping).
//
// Conversation key derivation mirrors agy-proxy's getSessionKey, with one
// change: the automatic multi-turn key hashes the first USER message,
// not messages[0] — OpenCode's messages[0] is its identical system
// prompt for every session, which would collide all conversations.
// Session map: SHARED with claude-code-proxy (mirrored key space), so
// both proxies must resolve to the SAME path. It lives in the shared
// CLI-proxy state root, not beside the script: bin/ is the tracked
// checkout, and every write there dirtied the primary clone and blocked
// pm-git-sync. #3067 completes the state-dir move of #2782, which
// covered the WORKDIR state (bin/work) but missed this PROXY_DIR anchor.
const SESSION_FILE =
  process.env.COMMANDCODE_PROXY_SESSION_FILE ||
  path.join(
    process.env.XDG_STATE_HOME || path.join(os.homedir(), '.local', 'state'),
    'cli-proxies',
    'sessions.json'
  );
fs.mkdirSync(path.dirname(SESSION_FILE), { recursive: true });
const MAX_SESSIONS = 200;
const sessionStore = new Map();

const saveSessions = () => {
  try {
    const obj = {};
    for (const [k, v] of sessionStore) obj[k] = v;
    fs.writeFileSync(`${SESSION_FILE}.tmp`, JSON.stringify(obj, null, 2) + '\n');
    fs.renameSync(`${SESSION_FILE}.tmp`, SESSION_FILE);
  } catch (err) {
    console.warn(`[commandcode-proxy] Failed to persist session store: ${err.message}`);
  }
};

const loadSessions = () => {
  try {
    const obj = JSON.parse(fs.readFileSync(SESSION_FILE, 'utf-8'));
    for (const [k, v] of Object.entries(obj)) {
      if (v && typeof v.sessionId === 'string') sessionStore.set(k, v);
    }
    if (sessionStore.size > 0) {
      console.log(`[commandcode-proxy] Loaded ${sessionStore.size} warm-session mapping(s) from ${SESSION_FILE}`);
    }
  } catch {}
};
loadSessions();

const pruneSessionStore = () => {
  if (sessionStore.size <= MAX_SESSIONS) return;
  const entries = Array.from(sessionStore.entries())
    .sort((a, b) => (a[1].updatedAt || 0) - (b[1].updatedAt || 0));
  for (const [key] of entries.slice(0, entries.length - MAX_SESSIONS)) {
    sessionStore.delete(key);
  }
};

const messageText = (m) => {
  if (!m) return '';
  if (typeof m.content === 'string') return m.content;
  if (Array.isArray(m.content)) return m.content.map(c => c.text || '').join('\n');
  return '';
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
    return explicitId.trim();
  }

  if (body.user && typeof body.user === 'string' && body.user.trim()) {
    return `user:${body.user.trim()}`;
  }

  if (Array.isArray(body.messages) && body.messages.length > 1) {
    const firstUser = body.messages.find(m => m.role === 'user');
    const firstContent = firstUser ? messageText(firstUser) : '';
    if (firstContent.length > 0) {
      const hash = crypto.createHash('sha256').update(firstContent).digest('hex').slice(0, 16);
      return `turn0:${hash}`;
    }
  }

  return null;
};

const extractLatestUserPrompt = (messages) => {
  if (!Array.isArray(messages) || messages.length === 0) return '';
  const lastUserMsg = [...messages].reverse().find(m => m.role === 'user');
  return lastUserMsg ? messageText(lastUserMsg) : '';
};

// One stable workdir per conversation key: the CLI keys its on-disk
// project state by cwd slug, so a stable cwd keeps a conversation's
// transcript in a single ~/.commandcode/projects/<slug>/ directory
// instead of scattering one project dir per request.
const sessionWorkdirFor = (key) => {
  const hash = crypto.createHash('sha256').update(key).digest('hex').slice(0, 16);
  const dir = path.join(WORKROOT, 'sessions', hash);
  fs.mkdirSync(dir, { recursive: true });
  return dir;
};



// ---------------------------------------------------------------------------
// Resident ACP engine (phase 2, Drew's requirement: "whatever saves
// tokens and reduces per call binary load/boot time").
//
// ONE resident `commandcode acp` process is held by this proxy and driven
// over stdio JSON-RPC (Agent Client Protocol — initialize, session/new,
// session/load, session/set_model, session/set_mode, session/prompt,
// session/close, session/cancel; streamed replies arrive as
// session/update agent_message_chunk notifications). Verified live
// 2026-10-24 against cmd v1.88.0: initialize ~7.4s once, session/new
// ~1.1s, turn 1 prompt 4.0s, turn 2 prompt 1.8s with per-turn usage
// cacheReadTokens 21632/21778 (~99% of input cache-read), codeword
// recalled from the resident session after sending only the new turn.
//
// Design notes:
// - One `cmd acp` process serves ONE project directory (upstream rule),
//   so every thread lives in ACP_HOME below; a conversation's transcript
//   persists at ~/.commandcode/projects/<slug-of-ACP_HOME>/<id>.jsonl.
// - Per conversation key (same derivation as the warm-session layer) the
//   proxy keeps key -> { sessionId, model, updatedAt } in sessions.json.
//   A live thread is reused directly; after a proxy/agent restart a
//   mapped thread is re-attached with session/load (history replay
//   updates are dropped — no prompt handler is registered during load).
// - Per call only the newest user turn is sent for an existing thread;
//   a brand-new thread gets the full flattened history once.
// - Threads are serialized per key (one prompt at a time per thread);
//   different threads may prompt concurrently.
// - ANY ACP failure returns { failed: true } and the caller falls back
//   to the spawn-per-request attempt chain (resume-by-transcript, then
//   fresh) — the v2 behavior remains the safety net.
const ACP_HOME = path.join(WORKROOT, 'acp-project');
fs.mkdirSync(ACP_HOME, { recursive: true });

const acp = {
  proc: null,
  ready: null,
  nextId: 1,
  pending: new Map(),       // rpc id -> { resolve, reject }
  updateHandlers: new Map(),// sessionId -> fn(update)
  liveSessions: new Map(),  // sessionId -> { model, mode, lastUsed }
  locks: new Map(),         // conversation key -> promise chain
  starts: 0,

  spawnProc() {
    const child = spawn(CC_BIN, ['acp'], {
      cwd: ACP_HOME,
      env: childEnv(),
      stdio: ['pipe', 'pipe', 'pipe']
    });
    this.proc = child;
    this.starts += 1;
    console.log(`[commandcode-proxy] ACP resident process started (pid ${child.pid}, start #${this.starts})`);

    let lineBuffer = '';
    child.stdout.on('data', chunk => {
      lineBuffer += chunk.toString('utf-8');
      const lines = lineBuffer.split('\n');
      lineBuffer = lines.pop();
      for (const line of lines) this.handleLine(line);
    });
    child.stderr.on('data', chunk => {
      const msg = chunk.toString('utf-8');
      if (msg.trim().length > 0) console.warn(`[acp stderr] ${msg.trim().slice(0, 500)}`);
    });
    child.on('error', err => console.error(`[commandcode-proxy] ACP process error: ${err.message}`));
    child.on('close', (code, signal) => {
      console.error(`[commandcode-proxy] ACP resident process exited (code ${code}, signal ${signal}) — in-flight prompts failed, next request restarts it`);
      if (this.proc === child) {
        this.proc = null;
        this.ready = null;
        this.liveSessions.clear();
        const err = new Error(`ACP process exited (code ${code})`);
        for (const [, p] of this.pending) p.reject(err);
        this.pending.clear();
        this.updateHandlers.clear();
      }
    });
  },

  write(obj) {
    if (!this.proc || !this.proc.stdin.writable) throw new Error('ACP process not running');
    this.proc.stdin.write(JSON.stringify(obj) + '\n');
  },

  handleLine(line) {
    const trimmed = line.trim();
    if (!trimmed || !trimmed.startsWith('{')) return;
    let msg;
    try { msg = JSON.parse(trimmed); } catch { return; }

    // Response to one of our requests
    if (msg.id !== undefined && !msg.method && this.pending.has(msg.id)) {
      const p = this.pending.get(msg.id);
      this.pending.delete(msg.id);
      if (msg.error) p.reject(new Error(`ACP ${msg.error.code || ''} ${msg.error.message || JSON.stringify(msg.error)}`.trim()));
      else p.resolve(msg.result);
      return;
    }

    // Agent -> client request (e.g. session/request_permission)
    if (msg.method && msg.id !== undefined) {
      if (msg.method === 'session/request_permission') {
        const opts = (msg.params && msg.params.options) || [];
        const pick = opts.find(o => o.kind === 'allow_once' || o.kind === 'allow_always')
          || opts.find(o => /allow/i.test(o.optionId || ''))
          || opts[0];
        try {
          this.write({ jsonrpc: '2.0', id: msg.id, result: { outcome: { outcome: 'selected', optionId: pick ? pick.optionId : 'allow_once' } } });
        } catch {}
      } else {
        // We declared fs/terminal capabilities false at initialize, so
        // other client services should never be called. Refuse politely.
        try {
          this.write({ jsonrpc: '2.0', id: msg.id, error: { code: -32601, message: `client service not supported: ${msg.method}` } });
        } catch {}
      }
      return;
    }

    // Notification: session/update streams prompt output
    if (msg.method === 'session/update' && msg.params) {
      const handler = this.updateHandlers.get(msg.params.sessionId);
      if (handler && msg.params.update) {
        try { handler(msg.params.update); } catch {}
      }
    }
  },

  request(method, params, timeoutMs = 60000) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`ACP ${method} timed out after ${timeoutMs}ms`));
      }, timeoutMs);
      timer.unref();
      this.pending.set(id, {
        resolve: (v) => { clearTimeout(timer); resolve(v); },
        reject: (e) => { clearTimeout(timer); reject(e); }
      });
      try {
        this.write({ jsonrpc: '2.0', id, method, params });
      } catch (err) {
        clearTimeout(timer);
        this.pending.delete(id);
        reject(err);
      }
    });
  },

  notify(method, params) {
    try { this.write({ jsonrpc: '2.0', method, params }); } catch {}
  },

  async ensureReady() {
    if (this.ready) return this.ready;
    this.ready = (async () => {
      this.spawnProc();
      const init = await this.request('initialize', {
        protocolVersion: 1,
        clientCapabilities: { fs: { readTextFile: false, writeTextFile: false }, terminal: false },
        clientInfo: { name: 'commandcode-proxy', version: '3.0.0' }
      }, 90000);
      console.log(`[commandcode-proxy] ACP initialized: ${(init.agentInfo && init.agentInfo.name) || 'agent'} ${(init.agentInfo && init.agentInfo.version) || ''}, loadSession=${Boolean(init.agentCapabilities && init.agentCapabilities.loadSession)}`);
      return init;
    })();
    this.ready.catch(() => { this.ready = null; });
    return this.ready;
  },

  async getOrCreateThread(mappingKey, model, ephemeral, mode) {
    await this.ensureReady();
    const stored = ephemeral ? null : sessionStore.get(mappingKey);
    let sessionId = null;
    let resumed = false;

    if (stored && stored.sessionId) {
      if (this.liveSessions.has(stored.sessionId)) {
        sessionId = stored.sessionId;
        resumed = true;
      } else {
        try {
          await this.request('session/load', { sessionId: stored.sessionId, cwd: ACP_HOME, mcpServers: [] }, 60000);
          sessionId = stored.sessionId;
          resumed = true;
          this.liveSessions.set(sessionId, { model: stored.model || null, mode: null, lastUsed: Date.now() });
          console.log(`[commandcode-proxy] ACP thread re-attached via session/load: ${sessionId} (key ${mappingKey})`);
        } catch (err) {
          console.warn(`[commandcode-proxy] ACP session/load failed for ${stored.sessionId} (${err.message}) — opening a new thread`);
        }
      }
    }

    if (!sessionId) {
      const created = await this.request('session/new', { cwd: ACP_HOME, mcpServers: [] }, 60000);
      sessionId = created.sessionId;
      this.liveSessions.set(sessionId, { model: null, mode: mode === 'dont-ask' ? null : 'default', lastUsed: Date.now() });
      if (!ephemeral) {
        sessionStore.set(mappingKey, { sessionId, model: null, updatedAt: Date.now() });
        pruneSessionStore();
        saveSessions();
      }
      console.log(`[commandcode-proxy] ACP thread created: ${sessionId} (key ${mappingKey}${ephemeral ? ', ephemeral' : ''})`);
    }

    const live = this.liveSessions.get(sessionId);
    if (live) {
      live.lastUsed = Date.now();
      // Permission posture is per-request, not per-thread: chat turns run
      // 'dont-ask' (fail-closed, text answers only); agency turns run the
      // session default mode (tools allowed; any permission prompt the CLI
      // sends is auto-allowed by handleLine). 'yolo' is NOT a valid ACP
      // modeId (verified: Unknown mode) — only the CLI flag.
      if (live.mode !== mode) {
        try { await this.request('session/set_mode', { sessionId, modeId: mode }, 30000); live.mode = mode; }
        catch (err) { console.warn(`[commandcode-proxy] ACP set_mode ${mode} failed (${err.message}) — continuing`); }
      }
      if (model && live.model !== model) {
        await this.request('session/set_model', { sessionId, modelId: model }, 30000);
        live.model = model;
        if (!ephemeral) {
          const cur = sessionStore.get(mappingKey);
          if (cur && cur.sessionId === sessionId) { cur.model = model; cur.updatedAt = Date.now(); saveSessions(); }
        }
      }
    }
    return { sessionId, resumed };
  },

  promptThread(sessionId, text, onDelta, timeoutMs) {
    return new Promise((resolve, reject) => {
      let fullContent = '';
      const handler = (update) => {
        if (update.sessionUpdate === 'agent_message_chunk' && update.content && update.content.type === 'text' && typeof update.content.text === 'string') {
          fullContent += update.content.text;
          if (onDelta) { try { onDelta(update.content.text); } catch {} }
        }
      };
      this.updateHandlers.set(sessionId, handler);
      const timer = setTimeout(() => {
        this.updateHandlers.delete(sessionId);
        this.notify('session/cancel', { sessionId });
        reject(new Error(`ACP prompt timed out after ${timeoutMs}ms`));
      }, timeoutMs);
      timer.unref();

      this.request('session/prompt', { sessionId, prompt: [{ type: 'text', text }] }, timeoutMs + 30000)
        .then(result => {
          clearTimeout(timer);
          this.updateHandlers.delete(sessionId);
          const usage = (result && result._meta && result._meta.usage) || (result && result.usage) || {};
          resolve({
            fullContent,
            promptTokens: typeof usage.inputTokens === 'number' ? usage.inputTokens : 0,
            completionTokens: typeof usage.outputTokens === 'number' ? usage.outputTokens : 0,
            cacheReadTokens: typeof usage.cacheReadTokens === 'number' ? usage.cacheReadTokens : 0,
            stopReason: result ? result.stopReason : null
          });
        })
        .catch(err => {
          clearTimeout(timer);
          this.updateHandlers.delete(sessionId);
          reject(err);
        });
    });
  },

  // Run one chat turn on the resident engine. Returns the same result
  // shape as the spawn-per-request attempts: { failed, detail, ... }.
  // agency: the OpenAI request carried tools — run the CLI in tool-using
  // mode (default session mode) with the agency prompt; otherwise chat
  // mode ('dont-ask', text answer only).
  async runChat({ sessionKey, model, firstPrompt, deltaPrompt, ephemeral, onDelta, agency }) {
    const lockKey = ephemeral ? `ephemeral:${crypto.randomUUID()}` : sessionKey;
    const prev = this.locks.get(lockKey) || Promise.resolve();
    const run = prev.then(
      () => this.runChatLocked({ sessionKey, model, firstPrompt, deltaPrompt, ephemeral, onDelta, agency, lockKey }),
      () => this.runChatLocked({ sessionKey, model, firstPrompt, deltaPrompt, ephemeral, onDelta, agency, lockKey })
    );
    this.locks.set(lockKey, run.catch(() => {}));
    return run;
  },

  async runChatLocked({ sessionKey, model, firstPrompt, deltaPrompt, ephemeral, onDelta, agency }) {
    let thread = null;
    try {
      thread = await this.getOrCreateThread(ephemeral ? null : sessionKey, model, ephemeral, agency ? 'default' : 'dont-ask');
      const text = thread.resumed && deltaPrompt && deltaPrompt.trim().length > 0 ? deltaPrompt : firstPrompt;
      console.log(`[commandcode-proxy] ACP prompt (${agency ? 'agency' : 'chat'}, ${thread.resumed ? 'delta' : 'full'}, ${text.length} chars) on thread ${thread.sessionId}`);
      const r = await this.promptThread(thread.sessionId, text, onDelta, TIMEOUT_MS);
      if (!r.fullContent || r.fullContent.trim().length === 0) {
        return { failed: true, detail: 'ACP prompt returned no text', fullContent: '', promptTokens: 0, completionTokens: 0, capturedSessionId: thread.sessionId };
      }
      const promptTokens = r.promptTokens > 0 ? r.promptTokens : Math.max(1, Math.ceil(text.length / 4));
      const completionTokens = r.completionTokens > 0 ? r.completionTokens : Math.max(1, Math.ceil(r.fullContent.length / 4));
      return { failed: false, detail: null, fullContent: r.fullContent, promptTokens, completionTokens, cacheReadTokens: r.cacheReadTokens || 0, capturedSessionId: thread.sessionId };
    } catch (err) {
      return { failed: true, detail: `ACP failed: ${err.message}`, fullContent: '', promptTokens: 0, completionTokens: 0, capturedSessionId: thread ? thread.sessionId : null };
    } finally {
      if (ephemeral && thread && thread.sessionId) {
        this.liveSessions.delete(thread.sessionId);
        try { await this.request('session/close', { sessionId: thread.sessionId }, 10000); } catch {}
      }
    }
  }
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
      service: 'commandcode-proxy',
      port: PORT,
      binary: CC_BIN,
      defaultModel: DEFAULT_MODEL,
      endpoint: '/v1/chat/completions',
      engine: 'acp-resident',
      acp: {
        running: Boolean(acp.proc),
        pid: acp.proc ? acp.proc.pid : null,
        starts: acp.starts,
        liveThreads: acp.liveSessions.size,
        storedMappings: sessionStore.size
      }
    }, null, 2));
    return;
  }

  // Models catalog
  if (req.method === 'GET' && (pathname === '/v1/models' || pathname === '/models')) {
    const now = Math.floor(Date.now() / 1000);
    const data = getServedModels().map(id => ({
      id,
      object: 'model',
      created: now,
      owned_by: 'commandcode'
    }));

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

      if (typeof body !== 'object' || body === null || Array.isArray(body)) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: 'Invalid JSON body: expected an object' } }));
        return;
      }

      const stream = Boolean(body.stream);
      const model = resolveModel(body.model);
      const chatId = `chatcmpl-commandcode-${crypto.randomUUID()}`;
      const created = Math.floor(Date.now() / 1000);
      const sessionKey = getSessionKey(req, body);
      const known = sessionKey ? sessionStore.get(sessionKey) : null;
      // Agency: the request carries tools, i.e. OpenCode wants an agentic
      // turn (its agent loop only sends tools when it wants tool calls).
      // The CLI then does the task with its own tools; chat turns stay
      // tool-free. Every `opencode run` (build agent) sends tools, so
      // opencode agent runs are agency turns by design.
      const agency = Array.isArray(body.tools) && body.tools.length > 0;
      const agencyWorkdir = sessionKey
        ? sessionWorkdirFor(sessionKey)
        : path.join(WORKROOT, `req-${crypto.randomUUID()}`);
      // Keyed workdirs are mkdir'd by sessionWorkdirFor; ephemeral chat
      // turns need no directory at all (the spawn fallback creates and
      // cleans its own). Only agency turns need the dir to exist, because
      // the agency prompt names it as the file-work root.
      if (agency) fs.mkdirSync(agencyWorkdir, { recursive: true });
      const toolManifest = agency
        ? body.tools.map(t => {
            const fn = t.function || t;
            const name = fn.name || t.type || 'tool';
            const desc = (fn.description || '').slice(0, 200);
            return `${name}${desc ? ` (${desc})` : ''}`;
          }).join('; ').slice(0, 2000)
        : '';
      const fullPrompt = formatPrompt(body.messages, agency, agencyWorkdir, toolManifest);

      if (!fullPrompt || fullPrompt.trim().length === 0) {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: 'Messages cannot be empty' } }));
        return;
      }

      // Attempt chain (warm-session pattern, issue #480): resume the
      // mapped CLI session with only the newest user turn when we have
      // one; a full-history fresh run is always the final fallback.
      const attempts = [];
      if (known && known.sessionId) {
        const latest = extractLatestUserPrompt(body.messages);
        if (latest.trim().length > 0) {
          const workdir = sessionWorkdirFor(sessionKey);
          attempts.push({
            mode: 'resume',
            // Agency resume: the raw latest turn lacks the agency
            // instruction — re-wrap it so the CLI runs tool-enabled with
            // the workdir steer even on the delta path.
            prompt: agency
              ? formatPrompt([{ role: 'user', content: latest }], true, workdir, toolManifest)
              : latest,
            sessionId: known.sessionId,
            workdir,
            persist: true
          });
        }
      }
      attempts.push({
        mode: 'fresh',
        prompt: fullPrompt,
        sessionId: null,
        workdir: sessionKey ? sessionWorkdirFor(sessionKey) : agencyWorkdir,
        persist: Boolean(sessionKey)
      });

      if (stream) {
        res.writeHead(200, {
          'Content-Type': 'text/event-stream; charset=utf-8',
          'Cache-Control': 'no-cache, no-transform',
          'Connection': 'keep-alive',
          'X-Accel-Buffering': 'no'
        });
      }

      let currentChild = null;
      let emittedDelta = false;
      res.on('close', () => {
        if (!res.writableEnded && currentChild) {
          try { currentChild.kill('SIGTERM'); } catch (_) {}
        }
      });

      const runAttempt = (attempt) => new Promise((resolve) => {
        fs.mkdirSync(attempt.workdir, { recursive: true });

        // Agency spawn: default permission mode (tools allowed — 'dont-ask'
        // fails them closed), --tools-all (headless -p withholds tools
        // otherwise), and a multi-turn budget. Chat spawn stays
        // single-turn dont-ask. cwd is the per-conversation workdir either
        // way; the CLI's own tools are unconfined there (fleet parity with
        // codex-proxy's danger-full-access), while the agency prompt
        // instructs it to stay inside the workdir.
        const args = [
          '-p',
          '--output-format', 'json',
          '-m', model,
          '--max-turns', agency ? AGENCY_MAX_TURNS : MAX_TURNS,
          '--skip-onboarding',
          '--no-auto-update',
          '-t'
        ];
        if (agency) args.push('--tools-all');
        else args.push('--permission-mode', 'dont-ask');
        if (attempt.sessionId) args.push('--session', attempt.sessionId);
        if (!attempt.persist) args.push('--no-session');

        console.log(`[commandcode-proxy] Spawning (${attempt.mode}${attempt.sessionId ? ` session=${attempt.sessionId}` : ''}): ${CC_BIN} ${args.join(' ')} (prompt ${attempt.prompt.length} chars)`);

        let child;
        try {
          child = spawn(CC_BIN, args, {
            cwd: attempt.workdir,
            env: childEnv(),
            stdio: ['pipe', 'pipe', 'pipe']
          });
        } catch (err) {
          resolve({ failed: true, detail: `Failed to spawn commandcode: ${err.message}`, fullContent: '', promptTokens: 0, completionTokens: 0, capturedSessionId: null });
          return;
        }
        currentChild = child;

        // Prompt goes in via stdin (`-p` with no query reads stdin).
        child.stdin.on('error', () => {});
        child.stdin.write(attempt.prompt);
        child.stdin.end();

        // Never hang forever: hard timeout per attempt.
        let timedOut = false;
        const timer = setTimeout(() => {
          timedOut = true;
          try { child.kill('SIGTERM'); } catch (_) {}
          setTimeout(() => { try { child.kill('SIGKILL'); } catch (_) {} }, 5000).unref();
        }, TIMEOUT_MS);
        timer.unref();

        let lineBuffer = '';
        let fullContent = '';
        let finalText = null;
        let promptTokens = Math.max(1, Math.ceil(attempt.prompt.length / 4));
        let completionTokens = 0;
        let stderrTail = '';
        let eventError = null;
        let capturedSessionId = null;

        const processJsonLine = (line) => {
          const trimmed = line.trim();
          if (!trimmed || !trimmed.startsWith('{')) return;

          try {
            const obj = JSON.parse(trimmed);
            const ev = obj.event || obj;
            if (!ev || typeof ev.type !== 'string') return;

            if (ev.type === 'run_start' && typeof ev.sessionId === 'string') {
              capturedSessionId = ev.sessionId;
            }

            // Assistant text delta (thinking_delta deliberately ignored)
            if (ev.type === 'text_delta' && typeof ev.delta === 'string') {
              const text = ev.delta;
              fullContent += text;

              if (stream && !res.writableEnded) {
                emittedDelta = true;
                const chunk = {
                  id: chatId,
                  object: 'chat.completion.chunk',
                  created,
                  model,
                  choices: [{
                    index: 0,
                    delta: { content: text },
                    finish_reason: null
                  }]
                };
                res.write(`data: ${JSON.stringify(chunk)}\n\n`);
              }
            }

            // Final result line: authoritative text + real token usage.
            // (result.nextState carries the full transcript/workspace
            // dump — never forwarded; its sessionId is captured.)
            if (ev.type === 'run_end' && ev.result) {
              if (typeof ev.result.finalText === 'string') finalText = ev.result.finalText;
              const usage = ev.result.usage || {};
              if (typeof usage.inputTokens === 'number' && usage.inputTokens > 0) promptTokens = usage.inputTokens;
              if (typeof usage.outputTokens === 'number' && usage.outputTokens > 0) completionTokens = usage.outputTokens;
              const nsId = ev.result.nextState && ev.result.nextState.sessionId;
              if (typeof nsId === 'string') capturedSessionId = nsId;
            }

            // Top-level final line: {"type":"result","finalText":...}
            if (ev.type === 'result' && typeof ev.finalText === 'string' && ev.finalText) {
              finalText = ev.finalText;
              const usage = ev.usage || {};
              if (typeof usage.inputTokens === 'number' && usage.inputTokens > 0) promptTokens = usage.inputTokens;
              if (typeof usage.outputTokens === 'number' && usage.outputTokens > 0) completionTokens = usage.outputTokens;
              if (typeof ev.sessionId === 'string') capturedSessionId = ev.sessionId;
            }

            if (ev.type === 'error' || ev.type === 'run_error') {
              // Shape: {"type":"run_error","error":{"name":"TransportError",
              // "message":"POST /alpha/generate → network error: fetch failed"}}
              const e = ev.error;
              eventError = (typeof ev.message === 'string' && ev.message)
                || (e && typeof e.message === 'string' && e.message)
                || (e && typeof e === 'string' && e)
                || (e ? JSON.stringify(e) : null)
                || 'commandcode reported an error event';
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
            console.warn(`[commandcode-proxy stderr] ${msg.trim()}`);
          }
        });

        child.on('close', (code, signal) => {
          clearTimeout(timer);
          if (lineBuffer.trim().length > 0) {
            processJsonLine(lineBuffer);
            lineBuffer = '';
          }
          if (!attempt.persist) {
            try { fs.rmSync(attempt.workdir, { recursive: true, force: true }); } catch (_) {}
          }

          // Prefer the run_end finalText if deltas missed any of it.
          if (finalText !== null && finalText.length >= fullContent.length) {
            fullContent = finalText;
          }

          const failed = timedOut || eventError !== null || (code !== 0 && fullContent.length === 0);
          if (failed) {
            let detail;
            if (timedOut) detail = `commandcode timed out after ${TIMEOUT_MS}ms`;
            else if (eventError) detail = String(eventError);
            else detail = (stderrTail.trim().split('\n').pop() || `commandcode exited with code ${code}${signal ? ` (signal ${signal})` : ''}`).trim();
            resolve({ failed: true, detail, fullContent, promptTokens, completionTokens, capturedSessionId });
            return;
          }

          if (completionTokens === 0 && fullContent.length > 0) {
            completionTokens = Math.max(1, Math.ceil(fullContent.length / 4));
          }
          resolve({ failed: false, detail: null, fullContent, promptTokens, completionTokens, capturedSessionId });
        });

        child.on('error', err => {
          clearTimeout(timer);
          if (!attempt.persist) {
            try { fs.rmSync(attempt.workdir, { recursive: true, force: true }); } catch (_) {}
          }
          resolve({ failed: true, detail: `CommandCode execution error: ${err.message}`, fullContent: '', promptTokens: 0, completionTokens: 0, capturedSessionId: null });
        });
      });

      (async () => {
        let result = null;

        // Phase 2 primary path: resident ACP engine (one binary boot
        // per service, delta-only turns on a persistent thread). Any
        // failure falls through to the spawn-per-request attempt chain.
        try {
          const latestTurn = extractLatestUserPrompt(body.messages);
          result = await acp.runChat({
            sessionKey,
            model,
            firstPrompt: fullPrompt,
            // Agency resume: re-wrap the delta turn so the workdir steer
            // is present even when only the newest turn is sent.
            deltaPrompt: agency && latestTurn.trim().length > 0
              ? formatPrompt([{ role: 'user', content: latestTurn }], true, agencyWorkdir, toolManifest)
              : latestTurn,
            ephemeral: !sessionKey,
            agency,
            onDelta: stream ? (text) => {
              if (!res.writableEnded) {
                emittedDelta = true;
                const chunk = {
                  id: chatId,
                  object: 'chat.completion.chunk',
                  created,
                  model,
                  choices: [{ index: 0, delta: { content: text }, finish_reason: null }]
                };
                res.write(`data: ${JSON.stringify(chunk)}\n\n`);
              }
            } : null
          });
          if (result && !result.failed) {
            console.log(`[commandcode-proxy] ACP turn ok (${result.promptTokens}+${result.completionTokens} tokens)`);
          } else if (result) {
            console.error(`[commandcode-proxy] ACP path failed, falling back to spawn-per-request: ${result.detail}`);
            result = null;
          }
        } catch (err) {
          console.error(`[commandcode-proxy] ACP path threw, falling back: ${err.message}`);
          result = null;
        }

        if (!result && !emittedDelta) {
        for (const attempt of attempts) {
          result = await runAttempt(attempt);
          if (!result.failed) {
            if (attempt.persist && sessionKey && result.capturedSessionId) {
              sessionStore.set(sessionKey, { sessionId: result.capturedSessionId, updatedAt: Date.now() });
              pruneSessionStore();
              saveSessions();
            }
            break;
          }
          console.error(`[commandcode-proxy] ${attempt.mode} attempt failed: ${result.detail}`);
          if (emittedDelta) break; // cannot retry mid-stream
          if (attempt.mode === 'resume' && sessionKey) {
            // Stale mapping: drop it so the fresh fallback re-creates it.
            sessionStore.delete(sessionKey);
            saveSessions();
          }
        }
        }

        if (res.writableEnded) return;

        if (!result || result.failed) {
          const detail = result ? result.detail : 'no attempt ran';
          if (stream) {
            // 200 + SSE headers are already on the wire: frame the error
            // as an SSE event and close with [DONE] (mirrors agy-proxy).
            res.write(`data: ${JSON.stringify({ error: { message: `CommandCode execution failed: ${detail}` } })}\n\n`);
            res.write('data: [DONE]\n\n');
            res.end();
          } else {
            res.writeHead(502, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ error: { message: `CommandCode execution failed: ${detail}` } }));
          }
          return;
        }

        if (stream) {
          const endChunk = {
            id: chatId,
            object: 'chat.completion.chunk',
            created,
            model,
            choices: [{
              index: 0,
              delta: {},
              finish_reason: 'stop'
            }],
            usage: {
              prompt_tokens: result.promptTokens,
              completion_tokens: result.completionTokens,
              total_tokens: result.promptTokens + result.completionTokens,
              ...(result.cacheReadTokens > 0 ? { prompt_tokens_details: { cached_tokens: result.cacheReadTokens } } : {})
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
            model,
            choices: [{
              index: 0,
              message: {
                role: 'assistant',
                content: result.fullContent
              },
              finish_reason: 'stop'
            }],
            usage: {
              prompt_tokens: result.promptTokens,
              completion_tokens: result.completionTokens,
              total_tokens: result.promptTokens + result.completionTokens,
              ...(result.cacheReadTokens > 0 ? { prompt_tokens_details: { cached_tokens: result.cacheReadTokens } } : {})
            }
          }, null, 2));
        }
      })();
    });
    return;
  }


  // Fallthrough 404
  res.writeHead(404, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ error: { message: `Route not found: ${req.method} ${pathname}` } }));
});

server.listen(PORT, HOST, () => {
  refreshLiveModels(); // warm the live model list in the background
  publishEndpoint();
  console.log(`[commandcode-proxy] OpenAI-compatible bridge listening on http://${HOST}:${PORT}`);
  console.log(`[commandcode-proxy] Resolved commandcode binary: ${CC_BIN}`);
  console.log(`[commandcode-proxy] Workroot: ${WORKROOT}`);
});
