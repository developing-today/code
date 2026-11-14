#!/usr/bin/env node
// muse agent for acpx — an ACP <-> MSP translator (code#2012, design #1721).
//
// OpenClaw/acpx speaks ACP (JSON-RPC 2.0, NDJSON over stdio) to this
// process; this adapter speaks MSP (the Muse CLI's own NDJSON JSON-RPC
// 2.0 session protocol) to a `muse serve` host it spawns on the laptop.
// It is a harness door: the agent loop, tools, and session store are the
// muse program's own — this file only translates protocols and enforces
// the #1721 build-blocking gates:
//
// - Plan-only by construction: the child is spawned with META_API_KEY
//   deleted from its environment (that key outranks the CLI's OAuth
//   login and is a separate pay-as-you-go wallet). There is no API-key
//   route anywhere in this adapter; auth is solely the CLI's laptop
//   OAuth login (~/.config/muse/auth.json), which the adapter never
//   reads, copies, or refreshes.
// - Per-session usage ledger: every turn (and every refusal) is appended
//   as one JSON line to the ledger file (default
//   ~/.local/state/muse-acp/ledger.jsonl), with the MSP
//   session/tokenUsage figures when the CLI reports them.
// - Pre-dispatch pool check: before a session starts, and again whenever
//   the last check is over an hour old, the freshest Meta pool reading
//   (default ~/atlas/meta-quota-export/latest.json, written by the
//   meta quota exporter) is consulted. A fresh (<= 60 min) reading with
//   any window at >= 100% refuses dispatch. A stale or absent reading is
//   not a known 100% and does not refuse; it is recorded in the ledger.
// - Per-session turn budget (MUSE_ACP_TURN_BUDGET, default 20): once a
//   session spends its budget, further prompts end with ACP stopReason
//   "max_turn_requests" and never reach the CLI.
// - Auth failure is terminal: an auth/login failure from the CLI ends
//   the session with the error "muse login required on laptop" — never
//   a silent retry, a route switch, or an interactive login attempt.
// - CLI drift is handled by probing `muse serve --help` /
//   `muse exec --help` at startup and gating optional flags on the
//   probed surface (the bin/muse-proxy.mjs pattern), never by
//   version-string checks.
//
// Transport: `muse serve` (warm host, durable sessions). `muse exec`
// is the degraded fallback only when serve fails before a turn's first
// output is committed; an exec-backed session stays on exec (it is
// stateless per turn) and its ledger lines say so (mode: "exec").

import { spawn, execFile } from "node:child_process";
import { createInterface } from "node:readline";
import { appendFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { randomFillSync } from "node:crypto";

const MUSE_BIN = process.env.MUSE_ACP_MUSE_BIN || "muse";
const PROVIDER = process.env.MUSE_ACP_PROVIDER || "meta";
const TURN_BUDGET = Number(process.env.MUSE_ACP_TURN_BUDGET || 20);
const POOL_FILE =
  process.env.MUSE_ACP_POOL_FILE || join(homedir(), "atlas", "meta-quota-export", "latest.json");
const POOL_MAX_AGE_MS = Number(process.env.MUSE_ACP_POOL_MAX_AGE_MS || 3_600_000);
const LEDGER_FILE =
  process.env.MUSE_ACP_LEDGER || join(homedir(), ".local", "state", "muse-acp", "ledger.jsonl");
const AUTH_FILE = join(homedir(), ".config", "muse", "auth.json");
const LOGIN_REQUIRED = "muse login required on laptop";
const AUTH_RE = /auth|login|unauthor|forbidden|\b401\b|\b403\b|oauth|credential/i;

// ---------------------------------------------------------------------------
// small utilities
// ---------------------------------------------------------------------------

function uuidv7() {
  const bytes = new Uint8Array(16);
  randomFillSync(bytes);
  const now = Date.now();
  bytes[0] = (now / 2 ** 40) & 0xff;
  bytes[1] = (now / 2 ** 32) & 0xff;
  bytes[2] = (now / 2 ** 24) & 0xff;
  bytes[3] = (now / 2 ** 16) & 0xff;
  bytes[4] = (now / 2 ** 8) & 0xff;
  bytes[5] = now & 0xff;
  bytes[6] = (bytes[6] & 0x0f) | 0x70;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function childEnv() {
  const env = { ...process.env };
  // Plan-only by construction (#1721 Q2/Q4): the PAYG key must never
  // outrank the CLI's OAuth login in the child.
  delete env.META_API_KEY;
  return env;
}

function ledger(entry) {
  try {
    mkdirSync(dirname(LEDGER_FILE), { recursive: true });
    appendFileSync(LEDGER_FILE, `${JSON.stringify({ ts: new Date().toISOString(), ...entry })}\n`);
  } catch {
    // The ledger is accounting, not a gate: a failed append must not
    // kill a session. Dispatch gates (pool/budget) are enforced in
    // memory before this point.
  }
}

// ---------------------------------------------------------------------------
// CLI capability probing (drift handling): probe --help, gate optional
// flags on the probed surface, never key off version strings.
// ---------------------------------------------------------------------------

let capsPromise = null;
function probeCaps() {
  if (!capsPromise) {
    const probe = (sub) =>
      new Promise((resolve) => {
        execFile(MUSE_BIN, [sub, "--help"], { env: childEnv(), timeout: 15_000 }, (err, stdout) =>
          resolve(err ? "" : String(stdout)),
        );
      });
    capsPromise = Promise.all([probe("serve"), probe("exec")]).then(([serveHelp, execHelp]) => ({
      serve: new Set([...serveHelp.matchAll(/--[a-z0-9-]+/g)].map((m) => m[0])),
      exec: new Set([...execHelp.matchAll(/--[a-z0-9-]+/g)].map((m) => m[0])),
    }));
  }
  return capsPromise;
}

async function buildArgs(sub, wanted) {
  const caps = await probeCaps();
  const set = caps[sub];
  const args = [sub];
  for (const { flag, value, optional } of wanted) {
    if (optional && !set.has(flag)) continue;
    args.push(flag);
    if (value !== undefined) args.push(value);
  }
  return args;
}

// ---------------------------------------------------------------------------
// pool check
// ---------------------------------------------------------------------------

// Returns { state: "ok" | "full" | "unknown", detail } — "full" only on a
// FRESH reading (<= POOL_MAX_AGE_MS old) with some window >= 100%.
function poolReading() {
  try {
    const doc = JSON.parse(readFileSync(POOL_FILE, "utf8"));
    const measuredAt = Date.parse(doc.measuredAt || "");
    if (!Number.isFinite(measuredAt)) return { state: "unknown", detail: "pool reading has no measuredAt" };
    const ageMs = Date.now() - measuredAt;
    if (ageMs > POOL_MAX_AGE_MS) {
      return { state: "unknown", detail: `pool reading is ${Math.round(ageMs / 60000)}m old (stale)` };
    }
    const windows = Array.isArray(doc.windows) ? doc.windows : [];
    const full = windows.find((w) => typeof w?.usedPct === "number" && w.usedPct >= 100);
    if (full) {
      return { state: "full", detail: `pool window "${full.name}" at ${full.usedPct}% (reading ${Math.round(ageMs / 60000)}m old)` };
    }
    return { state: "ok", detail: `pool ok (reading ${Math.round(ageMs / 60000)}m old)` };
  } catch (err) {
    return { state: "unknown", detail: `pool reading unreadable: ${err.message}` };
  }
}

// ---------------------------------------------------------------------------
// MSP serve host (one per adapter process, lazily started)
// ---------------------------------------------------------------------------

const host = {
  child: null,
  ready: null,
  seq: 0,
  pending: new Map(),
  listeners: new Set(),
  stderrTail: "",

  async ensure() {
    if (!this.ready) {
      this.ready = this._start().catch((err) => {
        this.ready = null;
        throw err;
      });
    }
    return this.ready;
  },

  async _start() {
    const args = await buildArgs("serve", [
      { flag: "--provider", value: PROVIDER },
      { flag: "--disable-request-user-input", optional: true },
    ]);
    const child = spawn(MUSE_BIN, args, { env: childEnv(), stdio: ["pipe", "pipe", "pipe"] });
    this.child = child;
    this.stderrTail = "";
    const rl = createInterface({ input: child.stdout });
    rl.on("line", (line) => this._onLine(line));
    child.stderr.on("data", (chunk) => {
      this.stderrTail = (this.stderrTail + chunk.toString()).slice(-2000);
    });
    child.on("error", (err) => this._onExit(`spawn error: ${err.message}`));
    child.on("close", (code, signal) => this._onExit(`serve host exited (code ${code}, signal ${signal})`));
    await this.rpc("initialize", {
      clientInfo: { name: "muse_acp_adapter", version: "0.1.0" },
      capabilities: { userInputDialogs: false },
    });
    this._write({ jsonrpc: "2.0", method: "initialized" });
  },

  _onExit(reason) {
    if (!this.child && this.pending.size === 0) return;
    this.child = null;
    this.ready = null;
    const err = new Error(reason);
    for (const p of this.pending.values()) p.reject(err);
    this.pending.clear();
    for (const fn of [...this.listeners]) {
      try {
        fn({ method: "host/died", params: { reason } });
      } catch { /* listener */ }
    }
  },

  _write(obj) {
    if (!this.child || !this.child.stdin.writable) throw new Error("serve host stdin is not writable");
    this.child.stdin.write(`${JSON.stringify(obj)}\n`);
  },

  _onLine(line) {
    const trimmed = line.trim();
    if (!trimmed.startsWith("{")) return;
    let msg;
    try {
      msg = JSON.parse(trimmed);
    } catch {
      return;
    }
    if (msg.id !== undefined && msg.method === undefined && this.pending.has(msg.id)) {
      const p = this.pending.get(msg.id);
      this.pending.delete(msg.id);
      if (msg.error) p.reject(new Error(`MSP ${msg.error.message || "error"} (code ${msg.error.code})`));
      else p.resolve(msg.result);
      return;
    }
    if (msg.id !== undefined && typeof msg.method === "string") {
      // Server-initiated request (approval / user input): sessions run
      // with approvalMode allowAll and request-user-input withheld, so
      // no decision is expected to follow; acknowledge presentation.
      try {
        this._write({ jsonrpc: "2.0", id: msg.id, result: {} });
      } catch { /* host gone */ }
      return;
    }
    if (typeof msg.method === "string") {
      for (const fn of [...this.listeners]) {
        try {
          fn(msg);
        } catch { /* listener */ }
      }
    }
  },

  rpc(method, params, timeoutMs = 30_000) {
    return new Promise((resolve, reject) => {
      const id = ++this.seq;
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`MSP ${method} timed out after ${timeoutMs}ms`));
      }, timeoutMs);
      this.pending.set(id, {
        resolve: (v) => {
          clearTimeout(timer);
          resolve(v);
        },
        reject: (e) => {
          clearTimeout(timer);
          reject(e);
        },
      });
      try {
        this._write({ jsonrpc: "2.0", id, method, params });
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

  async startSession(cwd) {
    const result = await this.rpc("session/start", {
      commandId: uuidv7(),
      approvalMode: "allowAll",
      workspaceRoots: [cwd],
    });
    const sessionId = result?.session?.sessionId;
    if (!sessionId) throw new Error("session/start returned no sessionId");
    return sessionId;
  },

  leave(sessionId) {
    if (!sessionId || !this.ready) return;
    this.rpc("session/leave", { commandId: uuidv7(), sessionId }, 5_000).catch(() => {});
  },
};

// One MSP turn on a loaded session. Streams text deltas to onDelta and
// resolves { usage } at turn/completed; rejects on any other terminal.
function runServeTurn({ sessionId, text, onDelta, isCancelled }) {
  return new Promise((resolve, reject) => {
    let turnId = null;
    let usage = null;
    let settled = false;
    const itemKinds = new Map();
    const settle = (fn, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      off();
      fn(value);
    };
    const timer = setTimeout(() => settle(reject, new Error("serve turn timed out after 600000ms")), 600_000);
    const off = host.onNotification((msg) => {
      const params = msg.params || {};
      if (params.sessionId !== sessionId && msg.method !== "host/died") return;
      switch (msg.method) {
        case "host/died":
          settle(reject, new Error(`serve host died mid-turn: ${params.reason}`));
          break;
        case "item/started":
          if (params.item?.itemId) itemKinds.set(params.item.itemId, params.item.kind);
          break;
        case "item/delta": {
          const kind = itemKinds.get(params.itemId);
          if (
            (params.field === undefined || params.field === "text") &&
            (kind === undefined || kind === "agentMessage") &&
            typeof params.delta === "string" &&
            !isCancelled()
          ) {
            try {
              onDelta(params.delta);
            } catch { /* client gone */ }
          }
          break;
        }
        case "session/tokenUsage":
          if (params.usage) {
            usage = {
              inputTokens: params.usage.inputTokens,
              outputTokens: params.usage.outputTokens,
              cacheReadTokens:
                typeof params.usage.cacheReadTokens === "number" ? params.usage.cacheReadTokens : 0,
            };
          }
          break;
        case "turn/completed":
          if (turnId && params.turnId !== turnId) break;
          if (params.terminal !== "completed") {
            settle(
              reject,
              new Error(`turn terminated '${params.terminal}'${params.reason ? `: ${params.reason}` : ""}`),
            );
          } else {
            settle(resolve, { usage });
          }
          break;
        default:
          break;
      }
    });
    host
      .rpc("turn/start", { commandId: uuidv7(), sessionId, input: [{ type: "text", text }] })
      .then((ack) => {
        turnId = ack?.turnId || turnId;
      })
      .catch((err) => settle(reject, err));
  });
}

// ---------------------------------------------------------------------------
// exec fallback (degraded, one-shot): `muse exec --json` emits the same
// MSP session events as JSONL. Used only when serve fails before a
// turn's first output is committed.
// ---------------------------------------------------------------------------

async function runExecTurn({ text, cwd, onDelta }) {
  const args = await buildArgs("exec", [
    { flag: "--json", optional: true },
    { flag: "--provider", value: PROVIDER },
  ]);
  args.push(text);
  return new Promise((resolve, reject) => {
    const child = spawn(MUSE_BIN, args, { env: childEnv(), cwd, stdio: ["ignore", "pipe", "pipe"] });
    let usage = null;
    let terminal = null;
    let stderrTail = "";
    const rl = createInterface({ input: child.stdout });
    rl.on("line", (line) => {
      let event;
      try {
        event = JSON.parse(line);
      } catch {
        return;
      }
      const payload = event.payload || {};
      if (event.payload_type === "run.output.delta" && typeof payload.text === "string") {
        try {
          onDelta(payload.text);
        } catch { /* client gone */ }
      } else if (event.payload_type === "run.usage" && Array.isArray(payload.models)) {
        const m = payload.models[0] || {};
        usage = {
          inputTokens: m.inputTokens ?? m.input_tokens,
          outputTokens: m.outputTokens ?? m.output_tokens,
          cacheReadTokens: m.cacheReadTokens ?? m.cached_tokens ?? 0,
        };
      } else if (typeof event.payload_type === "string" && event.payload_type.startsWith("run.terminal.")) {
        terminal = payload.terminal || event.payload_type.slice("run.terminal.".length);
      }
    });
    child.stderr.on("data", (chunk) => {
      stderrTail = (stderrTail + chunk.toString()).slice(-2000);
    });
    child.on("error", (err) => reject(err));
    child.on("close", (code) => {
      if (terminal && terminal !== "completed") {
        reject(new Error(`exec turn terminated '${terminal}'${stderrTail ? `: ${stderrTail.trim()}` : ""}`));
      } else if (code !== 0 && !terminal) {
        reject(new Error(`muse exec exited ${code}${stderrTail ? `: ${stderrTail.trim()}` : ""}`));
      } else {
        resolve({ usage });
      }
    });
  });
}

// ---------------------------------------------------------------------------
// ACP side
// ---------------------------------------------------------------------------

function send(obj) {
  process.stdout.write(`${JSON.stringify(obj)}\n`);
}
function sendResult(id, result) {
  send({ jsonrpc: "2.0", id, result });
}
function sendError(id, code, message) {
  send({ jsonrpc: "2.0", id, error: { code, message } });
}
function sessionUpdate(sessionId, update) {
  send({ jsonrpc: "2.0", method: "session/update", params: { sessionId, update } });
}

const sessions = new Map(); // acpSessionId -> session record
let lastPoolCheckAt = 0;
let lastPoolState = "unknown";

function freshPoolGate() {
  // Re-check at least hourly across the adapter's life; the reading
  // itself must be <= 60 min old to count as known.
  if (Date.now() - lastPoolCheckAt > POOL_MAX_AGE_MS || lastPoolCheckAt === 0) {
    const reading = poolReading();
    lastPoolCheckAt = Date.now();
    lastPoolState = reading.state;
    return reading;
  }
  return { state: lastPoolState, detail: "pool reading cached within the hour" };
}

function isAuthFailure(err) {
  return AUTH_RE.test(String(err?.message || err || "")) || AUTH_RE.test(host.stderrTail || "");
}

async function handleSessionNew(id, params) {
  const cwd = typeof params?.cwd === "string" && params.cwd ? params.cwd : process.cwd();
  if (!existsSync(AUTH_FILE)) {
    ledger({ kind: "refusal", reason: "no-login", detail: "auth.json absent" });
    return sendError(id, -32000, LOGIN_REQUIRED);
  }
  const pool = freshPoolGate();
  if (pool.state === "full") {
    ledger({ kind: "refusal", reason: "pool-full", detail: pool.detail });
    return sendError(id, -32000, `meta pool at capacity: ${pool.detail}; dispatch refused`);
  }
  try {
    await host.ensure();
    const mspSessionId = await host.startSession(cwd);
    sessions.set(mspSessionId, {
      mspSessionId,
      cwd,
      turns: 0,
      mode: "serve",
      dead: false,
      activeTurn: null,
    });
    ledger({ kind: "session-start", sessionId: mspSessionId, mode: "serve", pool: pool.state });
    return sendResult(id, { sessionId: mspSessionId });
  } catch (err) {
    if (isAuthFailure(err)) {
      ledger({ kind: "refusal", reason: "auth", detail: String(err.message || err) });
      return sendError(id, -32000, LOGIN_REQUIRED);
    }
    // Degraded fallback (#1721 Q3): serve failed before anything was
    // committed — open an exec-backed session instead.
    const execId = `exec-${uuidv7()}`;
    sessions.set(execId, { mspSessionId: null, cwd, turns: 0, mode: "exec", dead: false, activeTurn: null });
    ledger({ kind: "session-start", sessionId: execId, mode: "exec", pool: pool.state, serveError: String(err.message || err) });
    return sendResult(id, { sessionId: execId });
  }
}

async function handleSessionPrompt(id, params) {
  const sessionId = params?.sessionId;
  const session = sessions.get(sessionId);
  if (!session) return sendError(id, -32602, `unknown session ${sessionId}`);
  if (session.dead) return sendError(id, -32000, LOGIN_REQUIRED);
  const text = (Array.isArray(params?.prompt) ? params.prompt : [])
    .filter((b) => b && b.type === "text" && typeof b.text === "string")
    .map((b) => b.text)
    .join("\n");
  if (session.turns >= TURN_BUDGET) {
    ledger({ kind: "refusal", reason: "turn-budget", sessionId, turns: session.turns, budget: TURN_BUDGET });
    return sendResult(id, { stopReason: "max_turn_requests" });
  }
  const pool = freshPoolGate();
  if (pool.state === "full") {
    ledger({ kind: "refusal", reason: "pool-full", sessionId, detail: pool.detail });
    return sendError(id, -32000, `meta pool at capacity: ${pool.detail}; dispatch refused`);
  }
  const turnState = { cancelled: false };
  session.activeTurn = turnState;
  const onDelta = (delta) =>
    sessionUpdate(sessionId, {
      sessionUpdate: "agent_message_chunk",
      content: { type: "text", text: delta },
    });
  try {
    const { usage } =
      session.mode === "exec"
        ? await runExecTurn({ text, cwd: session.cwd, onDelta })
        : await runServeTurn({ sessionId: session.mspSessionId, text, onDelta, isCancelled: () => turnState.cancelled });
    session.turns += 1;
    ledger({
      kind: "turn",
      sessionId,
      turn: session.turns,
      mode: session.mode,
      inputTokens: usage?.inputTokens,
      outputTokens: usage?.outputTokens,
      cacheReadTokens: usage?.cacheReadTokens,
      stopReason: turnState.cancelled ? "cancelled" : "end_turn",
    });
    return sendResult(id, { stopReason: turnState.cancelled ? "cancelled" : "end_turn" });
  } catch (err) {
    if (isAuthFailure(err)) {
      session.dead = true;
      ledger({ kind: "refusal", reason: "auth", sessionId, detail: String(err.message || err) });
      return sendError(id, -32000, LOGIN_REQUIRED);
    }
    ledger({ kind: "turn-error", sessionId, mode: session.mode, detail: String(err.message || err) });
    return sendError(id, -32000, String(err.message || err));
  } finally {
    session.activeTurn = null;
  }
}

async function handleMessage(msg) {
  const { id, method, params } = msg;
  if (method === "initialize") {
    return sendResult(id, {
      protocolVersion: 1,
      agentCapabilities: { loadSession: false },
      agentInfo: { name: "muse-acp-adapter", title: "Muse (acpx muse agent)", version: "0.1.0" },
    });
  }
  if (method === "authenticate") {
    // The adapter owns no credentials (#1721 Q4): auth is the CLI's
    // laptop OAuth login, checked at session/new.
    return sendResult(id, {});
  }
  if (method === "session/new") return handleSessionNew(id, params);
  if (method === "session/prompt") return handleSessionPrompt(id, params);
  if (method === "session/cancel") {
    const session = sessions.get(params?.sessionId);
    if (session?.activeTurn) session.activeTurn.cancelled = true;
    return undefined;
  }
  if (id !== undefined) return sendError(id, -32601, `method not found: ${method}`);
  return undefined;
}

const rl = createInterface({ input: process.stdin });
rl.on("line", (line) => {
  const trimmed = line.trim();
  if (!trimmed.startsWith("{")) return;
  let msg;
  try {
    msg = JSON.parse(trimmed);
  } catch {
    return;
  }
  if (typeof msg.method === "string") {
    handleMessage(msg).catch((err) => {
      if (msg.id !== undefined) sendError(msg.id, -32603, String(err.message || err));
    });
  }
});
rl.on("close", () => {
  for (const session of sessions.values()) {
    if (session.mode === "serve" && session.mspSessionId) host.leave(session.mspSessionId);
  }
  setTimeout(() => process.exit(0), 250).unref();
});
