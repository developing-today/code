#!/usr/bin/env node
// OpenCode ACP-to-HTTP adapter (#1243).
//
// A single-purpose shim that speaks ACP (Agent Client Protocol,
// JSON-RPC 2.0 over stdio, newline-delimited) on one side and the
// OpenCode serve HTTP API on the other, so an ACP client (OpenClaw's
// acpx runtime) drives the fleet's ONE standing shared `opencode serve`
// instead of `opencode acp`, which starts a private server per process.
//
// Mapping (per #1243; v2 session API per #4614, probed live from the
// standing server's OpenAPI + event stream, task 6571):
//   ACP initialize            -> static capabilities (no server call)
//   ACP session/new           -> POST /api/session (cwd -> body
//                                location.directory; {data} envelope)
//   ACP session/prompt        -> POST /api/session/:id/prompt ({text,
//                                files}), then the GET /api/event SSE
//                                stream (v2 event model: session.text.*
//                                / session.reasoning.* deltas) is
//                                translated back into ACP session/update
//                                notifications; the prompt resolves on
//                                session.execution.succeeded
//   serve permission.asked    -> ACP session/request_permission; the
//                                client's choice is posted back to
//                                POST /api/session/:id/permission/
//                                :permId/reply ({decision})
//   ACP session/cancel        -> POST /api/session/:id/interrupt
//
// Config:
//   --server <url>            shared serve base URL
//                             (env OPENCODE_ACP_SERVER; default
//                             http://127.0.0.1:13420 — the standing
//                             shared server OpenChamber attaches to)
//   OPENCODE_SERVER_PASSWORD  when set, sent as HTTP basic auth
//                             (user "opencode"), matching serve's auth
//   OPENCODE_AUTH_FILE        JSON state file carrying the standing
//                             server's "password" field, read at
//                             runtime when the env var is unset
//                             (default ~/.local/state/opencode/
//                             service-prod.json — the server's own
//                             state file, #4615). The value is never
//                             logged or baked into tracked config.
//   OPENCODE_ACP_ADAPTER_DEBUG=1 -> diagnostics on stderr (stdout is
//                             the protocol channel and stays clean)
//
// The adapter spawns no server of its own; if the shared serve is
// unreachable, session/new fails with the connection error.

import { readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
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

// Resolve the standing server's password (#4615), mirroring the fleet
// pattern (pm-stack dispatch's OpenCodeClient, bin/openchamber-start.sh):
// the password lives in the server's own state file and is read at
// runtime — never baked into tracked config or the Nix store, never
// logged. Precedence: the OPENCODE_SERVER_PASSWORD env var, then the
// JSON state file named by OPENCODE_AUTH_FILE (default
// ~/.local/state/opencode/service-prod.json, field "password").
// Resolved lazily on first use and cached for the process lifetime
// (acpx launches a fresh adapter per gateway session).
//
// A missing state file just means no auth (a passwordless dev serve).
// A state file that exists but is malformed, or carries no password,
// warns once on stderr — the file path and the problem, never any
// credential value — and requests go out unauthenticated, so the
// serve's own 401 names the real problem instead of a silent hang.
let resolvedPassword;
let credentialResolved = false;
function serverPassword() {
  if (credentialResolved) return resolvedPassword;
  credentialResolved = true;
  const fromEnv = process.env.OPENCODE_SERVER_PASSWORD;
  if (fromEnv) {
    resolvedPassword = fromEnv;
    return resolvedPassword;
  }
  const authFile =
    process.env.OPENCODE_AUTH_FILE ||
    join(homedir(), '.local', 'state', 'opencode', 'service-prod.json');
  let raw;
  try {
    raw = readFileSync(authFile, 'utf8');
  } catch {
    debug(`no readable credential state file at ${authFile}; requests go unauthenticated`);
    return undefined;
  }
  try {
    const parsed = JSON.parse(raw);
    if (typeof parsed?.password === 'string' && parsed.password) {
      resolvedPassword = parsed.password;
      return resolvedPassword;
    }
    process.stderr.write(
      `[${AGENT_NAME}] credential state file ${authFile} carries no password; requests go unauthenticated\n`,
    );
  } catch {
    process.stderr.write(
      `[${AGENT_NAME}] credential state file ${authFile} is not valid JSON; requests go unauthenticated\n`,
    );
  }
  return undefined;
}

function authHeaders(extra = {}) {
  const headers = { ...extra };
  const password = serverPassword();
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
  if (!text) return null;
  const parsed = JSON.parse(text);
  // The v2 API wraps responses in a {data: …} envelope.
  if (parsed && typeof parsed === 'object' && 'data' in parsed) return parsed.data;
  return parsed;
}

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
      const res = await fetch(`${serverBase}/api/event`, {
        headers: authHeaders({ Accept: 'text/event-stream' }),
      });
      if (!res.ok || !res.body) throw new Error(`GET /api/event -> ${res.status}`);
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

function eventSessionId(data) {
  return data?.sessionID || null;
}

function streamDelta(sessionId, kind, delta) {
  if (typeof delta !== 'string' || delta.length === 0) return;
  sessionUpdate(sessionId, {
    sessionUpdate: kind === 'reasoning' ? 'agent_thought_chunk' : 'agent_message_chunk',
    content: { type: 'text', text: delta },
  });
}

const PERMISSION_OPTION_TO_RESPONSE = {
  allow_once: 'once',
  allow_always: 'always',
  reject_once: 'reject',
  reject_always: 'reject',
};

async function handlePermissionAsked(data) {
  const permission = data || {};
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
        title:
          permission.message ||
          `Permission requested: ${permission.action || 'tool'}${permission.resources?.length ? ` (${permission.resources.join(', ')})` : ''}`,
        kind: kindByType[permission.action] || 'other',
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
  try {
    await serveJson(`/api/session/${sessionId}/permission/${permissionId}/reply`, {
      method: 'POST',
      body: { decision: response },
    });
  } catch (err) {
    debug(`posting permission reply failed: ${err.message}`);
  }
}

function finishPrompt(sessionId, result, error) {
  const pending = pendingPrompts.get(sessionId);
  if (!pending) return;
  pendingPrompts.delete(sessionId);
  // A cancel marker belongs to the prompt it arrived during. Always clear
  // it when that prompt reaches a terminal event so a cancel racing
  // completion (or an error) cannot mislabel the next prompt (#4618).
  cancelledSessions.delete(sessionId);
  if (error) sendError(pending.requestId, -32603, error);
  else sendResult(pending.requestId, result);
}

function handleServeEvent(event) {
  const { type, data } = event;
  const sessionId = eventSessionId(data || {});
  switch (type) {
    case 'session.text.delta':
      if (sessionId && pendingPrompts.has(sessionId)) {
        streamDelta(sessionId, 'text', data.delta);
      }
      break;
    case 'session.reasoning.delta':
      if (sessionId && pendingPrompts.has(sessionId)) {
        streamDelta(sessionId, 'reasoning', data.delta);
      }
      break;
    case 'permission.asked':
      void handlePermissionAsked(data || {});
      break;
    case 'session.execution.succeeded':
      if (sessionId && pendingPrompts.has(sessionId)) {
        const wasCancelled = cancelledSessions.has(sessionId);
        cancelledSessions.delete(sessionId);
        finishPrompt(sessionId, { stopReason: wasCancelled ? 'cancelled' : 'end_turn' });
      }
      break;
    case 'session.execution.failed': {
      if (sessionId && pendingPrompts.has(sessionId)) {
        const message =
          data?.error?.message || data?.message || data?.error?.name || 'session execution failed';
        cancelledSessions.delete(sessionId);
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

// The v2 prompt body is { text, files } — there is no parts array.
// Text-bearing ACP blocks concatenate into `text`; binary/link blocks
// become file attachments by URI (data: URIs for embedded content).
function acpBlocksToPromptBody(blocks) {
  const texts = [];
  const files = [];
  for (const block of blocks || []) {
    if (!block || typeof block.type !== 'string') continue;
    switch (block.type) {
      case 'text':
        texts.push(block.text || '');
        break;
      case 'image':
        files.push({
          uri: `data:${block.mimeType || 'image/png'};base64,${block.data || ''}`,
          name: 'image',
        });
        break;
      case 'resource':
        if (typeof block.resource?.text === 'string') {
          texts.push(block.resource.text);
        } else if (typeof block.resource?.blob === 'string') {
          const mime = block.resource.mimeType || 'application/octet-stream';
          files.push({ uri: `data:${mime};base64,${block.resource.blob}`, name: 'resource' });
        }
        break;
      case 'resource_link':
        files.push({ uri: block.uri, name: block.name || 'link' });
        break;
      default:
        debug(`dropping unsupported ACP content block type: ${block.type}`);
        break;
    }
  }
  const body = { text: texts.join('\n') };
  if (files.length) body.files = files;
  return body;
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
    const session = await serveJson('/api/session', {
      method: 'POST',
      body: { location: { directory: cwd } },
    });
    if (!session?.id) throw new Error('serve POST /api/session returned no session id');
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
    const body = acpBlocksToPromptBody(params.prompt);
    // Register before posting so fast events are not missed; the reply is
    // sent by finishPrompt when the serve reports
    // session.execution.succeeded / .failed.
    pendingPrompts.set(sessionId, { requestId });
    try {
      await serveJson(`/api/session/${sessionId}/prompt`, {
        method: 'POST',
        body,
      });
    } catch (err) {
      pendingPrompts.delete(sessionId);
      cancelledSessions.delete(sessionId);
      throw err;
    }
    return undefined; // response deferred to finishPrompt
  },

  async 'session/cancel'(params) {
    const sessionId = params?.sessionId;
    if (!sessionId || !sessions.has(sessionId)) return;
    // A cancel arriving after the prompt already reached a terminal event
    // must not arm a stale marker for the next prompt (#4618). Prompt-state
    // checks here are synchronous with finishPrompt, rather than racy with
    // the abort POST outcome, because this prompt already holds a pending
    // ACP request; terminal-event cleanup is gated on the same map.
    if (!pendingPrompts.has(sessionId)) return;
    cancelledSessions.add(sessionId);
    try {
      await serveJson(`/api/session/${sessionId}/interrupt`, {
        method: 'POST',
        body: {},
      });
    } catch (err) {
      debug(`interrupt failed for ${sessionId}: ${err.message}`);
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
  // A line can be valid JSON without being a JSON-RPC message object:
  // `null` (the one value whose destructure throws) and other primitives
  // carry no id/method. Ignore them like the non-JSON lines the stdin
  // handler already skips (#6919) — destructuring `null` here escaped as
  // an unhandled rejection from the void-called dispatch and killed the
  // whole adapter.
  if (!message || typeof message !== 'object') {
    debug('ignoring non-object JSON line on stdin');
    return;
  }
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
