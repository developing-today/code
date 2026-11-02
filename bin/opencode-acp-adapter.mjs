#!/usr/bin/env node
// OpenCode ACP-to-HTTP adapter (#1243).
//
// A single-purpose shim that speaks ACP (Agent Client Protocol,
// JSON-RPC 2.0 over stdio, newline-delimited) on one side and the
// OpenCode serve HTTP API on the other, so an ACP client (OpenClaw's
// acpx runtime) drives the fleet's ONE standing shared `opencode serve`
// instead of `opencode acp`, which starts a private server per process.
//
// Mapping (per #1243):
//   ACP initialize            -> static capabilities (no server call)
//   ACP session/new           -> POST /session (cwd -> ?directory=)
//   ACP session/prompt        -> POST /session/:id/prompt_async, then the
//                                GET /event SSE stream is translated back
//                                into ACP session/update notifications;
//                                the prompt resolves on session.idle
//   serve permission.updated  -> ACP session/request_permission; the
//                                client's choice is posted back to
//                                POST /session/:id/permissions/:permId
//   ACP session/cancel        -> POST /session/:id/abort
//
// Config:
//   --server <url>            shared serve base URL
//                             (env OPENCODE_ACP_SERVER; default
//                             http://127.0.0.1:13420 — the standing
//                             shared server OpenChamber attaches to)
//   OPENCODE_SERVER_PASSWORD  when set, sent as HTTP basic auth
//                             (user "opencode"), matching serve's auth
//   OPENCODE_ACP_ADAPTER_DEBUG=1 -> diagnostics on stderr (stdout is
//                             the protocol channel and stays clean)
//
// The adapter spawns no server of its own; if the shared serve is
// unreachable, session/new fails with the connection error.

import { createInterface } from 'node:readline';
import process from 'node:process';

const PROTOCOL_VERSION = 1;
const AGENT_NAME = 'opencode-acp-adapter';
const AGENT_VERSION = '0.1.0';

const debug = (...args) => {
  if (process.env.OPENCODE_ACP_ADAPTER_DEBUG === '1') {
    process.stderr.write(`[${AGENT_NAME}] ${args.join(' ')}\n`);
  }
};

function parseServerUrl(argv) {
  let server = process.env.OPENCODE_ACP_SERVER || 'http://127.0.0.1:13420';
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === '--server' && argv[i + 1]) {
      server = argv[i + 1];
      i += 1;
    } else if (argv[i].startsWith('--server=')) {
      server = argv[i].slice('--server='.length);
    }
  }
  return server.replace(/\/+$/, '');
}

const serverBase = parseServerUrl(process.argv.slice(2));

function authHeaders(extra = {}) {
  const headers = { ...extra };
  const password = process.env.OPENCODE_SERVER_PASSWORD;
  if (password) {
    headers.Authorization = `Basic ${Buffer.from(`opencode:${password}`).toString('base64')}`;
  }
  return headers;
}

async function serveJson(pathname, { method = 'GET', body } = {}) {
  const res = await fetch(`${serverBase}${pathname}`, {
    method,
    headers: authHeaders(body === undefined ? {} : { 'Content-Type': 'application/json' }),
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new Error(`serve ${method} ${pathname} -> ${res.status} ${text.slice(0, 200)}`);
  }
  if (res.status === 204) return null;
  const text = await res.text();
  return text ? JSON.parse(text) : null;
}

const withDirectory = (pathname, directory) =>
  directory ? `${pathname}?directory=${encodeURIComponent(directory)}` : pathname;

// ---------------------------------------------------------------------------
// JSON-RPC peer over stdio
// ---------------------------------------------------------------------------

let nextOutgoingId = 1;
const pendingClientRequests = new Map(); // id -> { resolve, reject }

function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

function sendResult(id, result) {
  send({ jsonrpc: '2.0', id, result });
}

function sendError(id, code, message) {
  send({ jsonrpc: '2.0', id, error: { code, message } });
}

function sendNotification(method, params) {
  send({ jsonrpc: '2.0', method, params });
}

function requestClient(method, params) {
  const id = nextOutgoingId;
  nextOutgoingId += 1;
  return new Promise((resolve, reject) => {
    pendingClientRequests.set(id, { resolve, reject });
    send({ jsonrpc: '2.0', id, method, params });
  });
}

// ---------------------------------------------------------------------------
// Session state
// ---------------------------------------------------------------------------

const sessions = new Map(); // acpSessionId -> { directory }
const pendingPrompts = new Map(); // acpSessionId -> { requestId }
const cancelledSessions = new Set(); // acpSessionId
const partTextSent = new Map(); // partId -> chars of part.text already streamed

function sessionUpdate(sessionId, update) {
  sendNotification('session/update', { sessionId, update });
}

// ---------------------------------------------------------------------------
// Serve event stream (GET /event, SSE)
// ---------------------------------------------------------------------------

let eventStreamReady = null;
let eventStreamReadyResolve = null;
let shuttingDown = false;

function markEventStreamReady() {
  if (eventStreamReadyResolve) {
    eventStreamReadyResolve();
    eventStreamReadyResolve = null;
  }
}

async function ensureEventStream(timeoutMs = 5000) {
  if (!eventStreamReady) return;
  await Promise.race([
    eventStreamReady,
    new Promise((resolve) => setTimeout(resolve, timeoutMs)),
  ]);
}

async function runEventStream() {
  eventStreamReady = new Promise((resolve) => {
    eventStreamReadyResolve = resolve;
  });
  while (!shuttingDown) {
    try {
      const res = await fetch(`${serverBase}/event`, {
        headers: authHeaders({ Accept: 'text/event-stream' }),
      });
      if (!res.ok || !res.body) throw new Error(`GET /event -> ${res.status}`);
      markEventStreamReady();
      debug('event stream connected');
      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        let idx = buffer.indexOf('\n\n');
        while (idx >= 0) {
          const block = buffer.slice(0, idx);
          buffer = buffer.slice(idx + 2);
          handleSseBlock(block);
          idx = buffer.indexOf('\n\n');
        }
      }
      debug('event stream closed; reconnecting');
    } catch (err) {
      debug(`event stream error: ${err.message}; reconnecting`);
    }
    if (!shuttingDown) await new Promise((resolve) => setTimeout(resolve, 1000));
  }
}

function handleSseBlock(block) {
  const dataLines = block
    .split('\n')
    .filter((line) => line.startsWith('data:'))
    .map((line) => line.slice(5).trim());
  if (dataLines.length === 0) return;
  let event;
  try {
    event = JSON.parse(dataLines.join('\n'));
  } catch {
    return;
  }
  if (event && typeof event.type === 'string') handleServeEvent(event);
}

function eventSessionId(properties) {
  return (
    properties?.sessionID ||
    properties?.part?.sessionID ||
    properties?.info?.sessionID ||
    properties?.permission?.sessionID ||
    null
  );
}

function streamPartText(sessionId, part, delta) {
  if (!part || (part.type !== 'text' && part.type !== 'reasoning')) return;
  let chunk = '';
  if (typeof delta === 'string' && delta.length > 0) {
    chunk = delta;
    partTextSent.set(part.id, (partTextSent.get(part.id) || 0) + delta.length);
  } else if (typeof part.text === 'string') {
    const sent = partTextSent.get(part.id) || 0;
    chunk = part.text.slice(sent);
    partTextSent.set(part.id, part.text.length);
  }
  if (!chunk) return;
  sessionUpdate(sessionId, {
    sessionUpdate: part.type === 'reasoning' ? 'agent_thought_chunk' : 'agent_message_chunk',
    content: { type: 'text', text: chunk },
  });
}

const PERMISSION_OPTION_TO_RESPONSE = {
  allow_once: 'once',
  allow_always: 'always',
  reject_once: 'reject',
  reject_always: 'reject',
};

async function handlePermissionUpdated(properties) {
  const permission = properties?.permission ?? properties;
  const sessionId = permission?.sessionID;
  const permissionId = permission?.id;
  if (!sessionId || !permissionId || !sessions.has(sessionId)) return;
  const kindByType = { edit: 'edit', write: 'edit', bash: 'execute', external_directory: 'other' };
  let outcome;
  try {
    outcome = await requestClient('session/request_permission', {
      sessionId,
      toolCall: {
        toolCallId: permissionId,
        title: permission.title || `Permission requested: ${permission.type || 'tool'}`,
        kind: kindByType[permission.type] || 'other',
        status: 'pending',
        rawInput: permission.metadata ?? {},
      },
      options: [
        { optionId: 'allow_once', name: 'Allow once', kind: 'allow_once' },
        { optionId: 'allow_always', name: 'Allow always', kind: 'allow_always' },
        { optionId: 'reject_once', name: 'Reject', kind: 'reject_once' },
        { optionId: 'reject_always', name: 'Always reject', kind: 'reject_always' },
      ],
    });
  } catch (err) {
    debug(`request_permission failed (${err.message}); rejecting ${permissionId}`);
    outcome = null;
  }
  let response = 'reject';
  if (outcome && outcome.outcome?.outcome === 'selected') {
    response = PERMISSION_OPTION_TO_RESPONSE[outcome.outcome.optionId] || 'reject';
  }
  const { directory } = sessions.get(sessionId) || {};
  try {
    await serveJson(
      withDirectory(`/session/${sessionId}/permissions/${permissionId}`, directory),
      { method: 'POST', body: { response } },
    );
  } catch (err) {
    debug(`posting permission response failed: ${err.message}`);
  }
}

function finishPrompt(sessionId, result, error) {
  const pending = pendingPrompts.get(sessionId);
  if (!pending) return;
  pendingPrompts.delete(sessionId);
  partTextSent.clear();
  if (error) sendError(pending.requestId, -32603, error);
  else sendResult(pending.requestId, result);
}

function handleServeEvent(event) {
  const { type, properties } = event;
  const sessionId = eventSessionId(properties || {});
  switch (type) {
    case 'message.part.updated':
      if (sessionId && pendingPrompts.has(sessionId)) {
        streamPartText(sessionId, properties.part, properties.delta);
      }
      break;
    case 'permission.updated':
      void handlePermissionUpdated(properties || {});
      break;
    case 'session.idle':
      if (sessionId && pendingPrompts.has(sessionId)) {
        const wasCancelled = cancelledSessions.has(sessionId);
        cancelledSessions.delete(sessionId);
        finishPrompt(sessionId, { stopReason: wasCancelled ? 'cancelled' : 'end_turn' });
      }
      break;
    case 'session.error': {
      if (sessionId && pendingPrompts.has(sessionId)) {
        const message =
          properties?.error?.data?.message || properties?.error?.name || 'session error';
        finishPrompt(sessionId, null, `serve session error: ${message}`);
      }
      break;
    }
    default:
      break;
  }
}

// ---------------------------------------------------------------------------
// ACP method handlers
// ---------------------------------------------------------------------------

function acpBlocksToServeParts(blocks) {
  const parts = [];
  for (const block of blocks || []) {
    if (!block || typeof block.type !== 'string') continue;
    switch (block.type) {
      case 'text':
        parts.push({ type: 'text', text: block.text || '' });
        break;
      case 'image':
        parts.push({
          type: 'file',
          mime: block.mimeType || 'image/png',
          url: `data:${block.mimeType || 'image/png'};base64,${block.data || ''}`,
        });
        break;
      case 'resource':
        if (typeof block.resource?.text === 'string') {
          parts.push({ type: 'text', text: block.resource.text });
        } else if (typeof block.resource?.blob === 'string') {
          const mime = block.resource.mimeType || 'application/octet-stream';
          parts.push({ type: 'file', mime, url: `data:${mime};base64,${block.resource.blob}` });
        }
        break;
      case 'resource_link':
        parts.push({ type: 'file', mime: block.mimeType || '', url: block.uri });
        break;
      default:
        debug(`dropping unsupported ACP content block type: ${block.type}`);
        break;
    }
  }
  return parts;
}

const handlers = {
  initialize(params) {
    const requested = params?.protocolVersion;
    return {
      protocolVersion: requested === PROTOCOL_VERSION ? requested : PROTOCOL_VERSION,
      agentCapabilities: {
        loadSession: false,
        promptCapabilities: { image: true, audio: false, embeddedContext: true },
        mcpCapabilities: { http: false, sse: false },
      },
      agentInfo: { name: AGENT_NAME, title: 'OpenCode ACP-to-HTTP adapter (shared serve)', version: AGENT_VERSION },
      authMethods: [],
    };
  },

  async authenticate() {
    return {};
  },

  async 'session/new'(params) {
    const cwd = params?.cwd || process.cwd();
    await ensureEventStream();
    const session = await serveJson(withDirectory('/session', cwd), { method: 'POST', body: {} });
    if (!session?.id) throw new Error('serve POST /session returned no session id');
    sessions.set(session.id, { directory: cwd });
    debug(`session/new -> ${session.id} (directory ${cwd})`);
    return { sessionId: session.id };
  },

  async 'session/prompt'(params, requestId) {
    const sessionId = params?.sessionId;
    if (!sessionId || !sessions.has(sessionId)) {
      throw new Error(`unknown session: ${sessionId}`);
    }
    if (pendingPrompts.has(sessionId)) {
      throw new Error(`session ${sessionId} already has a prompt in flight`);
    }
    const { directory } = sessions.get(sessionId);
    const parts = acpBlocksToServeParts(params.prompt);
    // Register before posting so fast events are not missed; the reply is
    // sent by finishPrompt when the serve reports session.idle / error.
    pendingPrompts.set(sessionId, { requestId });
    try {
      await serveJson(withDirectory(`/session/${sessionId}/prompt_async`, directory), {
        method: 'POST',
        body: { parts },
      });
    } catch (err) {
      pendingPrompts.delete(sessionId);
      throw err;
    }
    return undefined; // response deferred to finishPrompt
  },

  async 'session/cancel'(params) {
    const sessionId = params?.sessionId;
    if (!sessionId || !sessions.has(sessionId)) return;
    cancelledSessions.add(sessionId);
    const { directory } = sessions.get(sessionId);
    try {
      await serveJson(withDirectory(`/session/${sessionId}/abort`, directory), {
        method: 'POST',
        body: {},
      });
    } catch (err) {
      debug(`abort failed for ${sessionId}: ${err.message}`);
    }
  },

  async 'session/set_mode'() {
    return {};
  },

  async 'session/set_model'() {
    return {};
  },
};

async function dispatch(message) {
  const { id, method, params } = message;
  const isRequest = id !== undefined && id !== null;
  if (!method) {
    // A response to one of our requests to the client.
    if (isRequest && pendingClientRequests.has(id)) {
      const pending = pendingClientRequests.get(id);
      pendingClientRequests.delete(id);
      if (message.error) pending.reject(new Error(message.error.message || 'client request failed'));
      else pending.resolve(message.result);
    }
    return;
  }
  const handler = handlers[method];
  if (!handler) {
    if (isRequest) sendError(id, -32601, `method not found: ${method}`);
    return;
  }
  try {
    const result = await handler(params || {}, id);
    if (isRequest && method !== 'session/prompt') sendResult(id, result ?? {});
    else if (isRequest && result !== undefined) sendResult(id, result);
  } catch (err) {
    if (isRequest) sendError(id, -32603, err.message || String(err));
  }
}

function main() {
  void runEventStream();
  const rl = createInterface({ input: process.stdin, crlfDelay: Infinity });
  rl.on('line', (line) => {
    const trimmed = line.trim();
    if (!trimmed) return;
    let message;
    try {
      message = JSON.parse(trimmed);
    } catch {
      debug(`ignoring non-JSON line on stdin: ${trimmed.slice(0, 120)}`);
      return;
    }
    void dispatch(message);
  });
  rl.on('close', () => {
    shuttingDown = true;
    process.exit(0);
  });
  process.on('SIGTERM', () => {
    shuttingDown = true;
    process.exit(0);
  });
  debug(`started; server=${serverBase}`);
}

main();
